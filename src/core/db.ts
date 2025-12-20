import { Database } from "bun:sqlite";
import path from "node:path";
import { z } from "zod";
import { logger } from "./logger";

export const AddonRecordSchema = z.object({
	id: z.number().optional(),
	name: z.string(),
	folder: z.string(),
	version: z.string().nullable(),
	git_commit: z.string().nullable().default(null),
	author: z.string().nullable(),
	interface: z.string().nullable(),
	url: z.string().nullable(),
	type: z.enum(["github", "tukui", "manual", "wowinterface"]),
	parent: z.string().nullable().default(null),
	install_date: z.string(),
	last_updated: z.string(),
});

export type AddonRecord = z.infer<typeof AddonRecordSchema>;

export class DatabaseManager {
	private db: Database;

	constructor(configDir: string) {
		const dbPath = path.join(configDir, "lemonup.db");
		logger.log("Database", `Opening database at ${dbPath}`);

		this.db = new Database(dbPath, { create: true });
		this.init();
	}

	private init() {
		this.db.run("PRAGMA journal_mode = WAL;");

		// biome-ignore lint/suspicious/noExplicitAny: user_version is a custom property from the query result
		const version = (this.db.query("PRAGMA user_version").get() as any)
			.user_version;

		logger.log("Database", `Current Schema Version: ${version}`);

		if (version < 1) {
			this.migrateToV1();
		}
		if (version < 2) {
			this.migrateToV2();
		}
	}

	private migrateToV1() {
		logger.log("Database", "Migrating to Schema V1...");

		this.db.transaction(() => {
			this.db.run(`
				CREATE TABLE IF NOT EXISTS addons (
					id INTEGER PRIMARY KEY AUTOINCREMENT,
					name TEXT NOT NULL,
					folder TEXT NOT NULL,
					version TEXT,
					git_commit TEXT,
					author TEXT,
					interface TEXT,
					url TEXT,
					type TEXT NOT NULL,
					install_date TEXT NOT NULL,
					last_updated TEXT NOT NULL
				);
			`);

			const tableInfo = this.db
				.query("PRAGMA table_info(addons)")
				// biome-ignore lint/suspicious/noExplicitAny: table_info result schema is dynamic
				.all() as any[];
			const hasGitCommit = tableInfo.some((col) => col.name === "git_commit");
			if (!hasGitCommit) {
				this.db.run("ALTER TABLE addons ADD COLUMN git_commit TEXT");
			}

			this.db.run(`
				CREATE UNIQUE INDEX IF NOT EXISTS idx_addons_folder ON addons(folder);
			`);

			this.db.run("PRAGMA user_version = 1");
		})();

		logger.log("Database", "Migration to Schema V1 complete");
	}

	private migrateToV2() {
		logger.log("Database", "Migrating to Schema V2...");
		this.db.transaction(() => {
			this.db.run("ALTER TABLE addons ADD COLUMN parent TEXT DEFAULT NULL");
			this.db.run("PRAGMA user_version = 2");
		})();
		logger.log("Database", "Migration to Schema V2 complete");
	}

	public getAll(): AddonRecord[] {
		return this.db
			.query("SELECT * FROM addons ORDER BY name ASC")
			.all() as AddonRecord[];
	}

	public getByFolder(folder: string): AddonRecord | null {
		return this.db
			.query("SELECT * FROM addons WHERE folder = $folder")
			.get({ $folder: folder }) as AddonRecord | null;
	}

	public addAddon(addon: AddonRecord): void {
		const data = AddonRecordSchema.parse(addon);
		const query = this.db.query(`
			INSERT INTO addons (name, folder, version, git_commit, author, interface, url, type, parent, install_date, last_updated)
			VALUES ($name, $folder, $version, $git_commit, $author, $interface, $url, $type, $parent, $install_date, $last_updated)
		`);

		query.run({
			$name: data.name,
			$folder: data.folder,
			$version: data.version,
			$git_commit: data.git_commit,
			$author: data.author,
			$interface: data.interface,
			$url: data.url,
			$type: data.type,
			$parent: data.parent,
			$install_date: data.install_date,
			$last_updated: data.last_updated,
		});
	}

	public updateAddon(folder: string, updates: Partial<AddonRecord>): void {
		const validUpdates = AddonRecordSchema.partial().parse(updates);

		const keys = Object.keys(validUpdates).filter(
			(k) => k !== "id" && k !== "folder",
		) as (keyof AddonRecord)[];
		if (keys.length === 0) return;

		const setClause = keys.map((k) => `${k} = $${k}`).join(", ");
		const query = this.db.query(
			`UPDATE addons SET ${setClause} WHERE folder = $folder`,
		);

		const params: Record<string, string | number | null | undefined> = {
			$folder: folder,
		};
		for (const key of keys) {
			params[`$${key}`] = validUpdates[key];
		}

		// biome-ignore lint/suspicious/noExplicitAny: parameters are mapped dynamically
		query.run(params as any);
	}

	public removeAddon(folder: string): void {
		this.db
			.query("DELETE FROM addons WHERE folder = $folder")
			.run({ $folder: folder });
	}

	public close() {
		this.db.close();
	}
}
