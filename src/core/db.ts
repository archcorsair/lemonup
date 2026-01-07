import { Database } from "bun:sqlite";
import path from "node:path";
import { z } from "zod";
import { logger } from "./logger";

export const AddonKind = z.enum(["addon", "library"]);
export type AddonKind = z.infer<typeof AddonKind>;

export const GameFlavor = z.enum(["retail", "classic", "cata"]);
export type GameFlavor = z.infer<typeof GameFlavor>;

export const AddonRecordSchema = z.object({
  id: z.number().optional(),
  name: z.string(),
  folder: z.string(),
  ownedFolders: z.array(z.string()).default([]),
  kind: AddonKind.default("addon"),
  kindOverride: z.boolean().default(false),
  flavor: GameFlavor.default("retail"),
  version: z.string().nullable(),
  git_commit: z.string().nullable().default(null),
  author: z.string().nullable(),
  interface: z.string().nullable(),
  url: z.string().nullable(),
  type: z.enum(["github", "tukui", "manual", "wowinterface", "wago"]),
  requiredDeps: z.array(z.string()).default([]),
  optionalDeps: z.array(z.string()).default([]),
  embeddedLibs: z.array(z.string()).default([]),
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

    // Schema V1 (new) uses ownedFolders, requiredDeps, etc.
    // Old schemas (V1/V2) had parent field - check for it and reset if found
    if (version === 0) {
      this.migrateToV1();
    } else {
      // Check if this is old schema with parent column
      const hasParent = this.db
        .query("PRAGMA table_info(addons)")
        .all()
        // biome-ignore lint/suspicious/noExplicitAny: PRAGMA result has dynamic schema
        .some((col: any) => col.name === "parent");

      if (hasParent) {
        logger.log(
          "Database",
          "Detected old schema with parent field, resetting...",
        );
        this.db.run("DROP TABLE IF EXISTS addons");
        this.db.run("PRAGMA user_version = 0");
        this.migrateToV1();
      }
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
					owned_folders TEXT NOT NULL DEFAULT '[]',
					kind TEXT NOT NULL DEFAULT 'addon',
					kind_override INTEGER NOT NULL DEFAULT 0,
					flavor TEXT NOT NULL DEFAULT 'retail',
					version TEXT,
					git_commit TEXT,
					author TEXT,
					interface TEXT,
					url TEXT,
					type TEXT NOT NULL,
					required_deps TEXT NOT NULL DEFAULT '[]',
					optional_deps TEXT NOT NULL DEFAULT '[]',
					embedded_libs TEXT NOT NULL DEFAULT '[]',
					install_date TEXT NOT NULL,
					last_updated TEXT NOT NULL
				);
			`);

      this.db.run(`
				CREATE UNIQUE INDEX IF NOT EXISTS idx_addons_folder ON addons(folder);
			`);

      this.db.run("PRAGMA user_version = 1");
    })();

    logger.log("Database", "Migration to Schema V1 complete");
  }

  // biome-ignore lint/suspicious/noExplicitAny: SQLite query results have dynamic schema
  private parseAddonRecord(row: any): AddonRecord {
    return {
      ...row,
      ownedFolders: row.owned_folders ? JSON.parse(row.owned_folders) : [],
      kind: row.kind || "addon",
      kindOverride: row.kind_override ? Boolean(row.kind_override) : false,
      flavor: row.flavor || "retail",
      requiredDeps: row.required_deps ? JSON.parse(row.required_deps) : [],
      optionalDeps: row.optional_deps ? JSON.parse(row.optional_deps) : [],
      embeddedLibs: row.embedded_libs ? JSON.parse(row.embedded_libs) : [],
    };
  }

  public getAll(): AddonRecord[] {
    const rows = this.db
      .query("SELECT * FROM addons ORDER BY name ASC")
      // biome-ignore lint/suspicious/noExplicitAny: SQLite query results have dynamic schema
      .all() as any[];
    return rows.map((row) => this.parseAddonRecord(row));
  }

  public getByFolder(folder: string): AddonRecord | null {
    const row = this.db
      .query("SELECT * FROM addons WHERE folder = $folder")
      // biome-ignore lint/suspicious/noExplicitAny: SQLite query results have dynamic schema
      .get({ $folder: folder }) as any;
    return row ? this.parseAddonRecord(row) : null;
  }

  public addAddon(addon: AddonRecord): void {
    const data = AddonRecordSchema.parse(addon);
    const query = this.db.query(`
			INSERT INTO addons (
				name, folder, owned_folders, kind, kind_override, flavor,
				version, git_commit, author, interface, url, type,
				required_deps, optional_deps, embedded_libs,
				install_date, last_updated
			)
			VALUES (
				$name, $folder, $owned_folders, $kind, $kind_override, $flavor,
				$version, $git_commit, $author, $interface, $url, $type,
				$required_deps, $optional_deps, $embedded_libs,
				$install_date, $last_updated
			)
		`);

    query.run({
      $name: data.name,
      $folder: data.folder,
      $owned_folders: JSON.stringify(data.ownedFolders),
      $kind: data.kind,
      $kind_override: data.kindOverride ? 1 : 0,
      $flavor: data.flavor,
      $version: data.version,
      $git_commit: data.git_commit,
      $author: data.author,
      $interface: data.interface,
      $url: data.url,
      $type: data.type,
      $required_deps: JSON.stringify(data.requiredDeps),
      $optional_deps: JSON.stringify(data.optionalDeps),
      $embedded_libs: JSON.stringify(data.embeddedLibs),
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

    // Map camelCase to snake_case for SQL columns
    const columnMap: Record<string, string> = {
      ownedFolders: "owned_folders",
      kindOverride: "kind_override",
      requiredDeps: "required_deps",
      optionalDeps: "optional_deps",
      embeddedLibs: "embedded_libs",
    };

    const setClause = keys
      .map((k) => `${columnMap[k] || k} = $${k}`)
      .join(", ");
    const query = this.db.query(
      `UPDATE addons SET ${setClause} WHERE folder = $folder`,
    );

    const params: Record<string, string | number | null | undefined> = {
      $folder: folder,
    };
    for (const key of keys) {
      const value = validUpdates[key];
      // Serialize arrays and booleans
      if (
        key === "ownedFolders" ||
        key === "requiredDeps" ||
        key === "optionalDeps" ||
        key === "embeddedLibs"
      ) {
        params[`$${key}`] = JSON.stringify(value);
      } else if (key === "kindOverride") {
        params[`$${key}`] = value ? 1 : 0;
      } else {
        params[`$${key}`] = value as string | number | null | undefined;
      }
    }

    // biome-ignore lint/suspicious/noExplicitAny: parameters are mapped dynamically
    query.run(params as any);
  }

  public removeAddon(folder: string): void {
    this.db
      .query("DELETE FROM addons WHERE folder = $folder")
      .run({ $folder: folder });
  }

  public getDependents(folder: string): AddonRecord[] {
    const rows = this.db
      .query(`
				SELECT * FROM addons
				WHERE required_deps LIKE '%' || $folder || '%'
				   OR optional_deps LIKE '%' || $folder || '%'
			`)
      // biome-ignore lint/suspicious/noExplicitAny: SQLite query results have dynamic schema
      .all({ $folder: folder }) as any[];
    return rows
      .map((row) => this.parseAddonRecord(row))
      .filter(
        (addon) =>
          addon.requiredDeps.includes(folder) ||
          addon.optionalDeps.includes(folder),
      );
  }

  public getRequiredDependents(folder: string): AddonRecord[] {
    const rows = this.db
      .query(`
				SELECT * FROM addons
				WHERE required_deps LIKE '%' || $folder || '%'
			`)
      // biome-ignore lint/suspicious/noExplicitAny: SQLite query results have dynamic schema
      .all({ $folder: folder }) as any[];
    return rows
      .map((row) => this.parseAddonRecord(row))
      .filter((addon) => addon.requiredDeps.includes(folder));
  }

  public getOwnerOf(folder: string): AddonRecord | null {
    const rows = this.db
      .query(`
				SELECT * FROM addons
				WHERE owned_folders LIKE '%' || $folder || '%'
			`)
      // biome-ignore lint/suspicious/noExplicitAny: SQLite query results have dynamic schema
      .all({ $folder: folder }) as any[];
    const matches = rows
      .map((row) => this.parseAddonRecord(row))
      .filter((addon) => addon.ownedFolders.includes(folder));
    return matches[0] ?? null;
  }

  public close() {
    this.db.close();
  }
}
