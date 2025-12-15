import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import Conf from "conf";
import { z } from "zod";

// --- Zod Schemas ---

export const RepositoryTypeSchema = z.enum(["github", "tukui"]);

export const RepositorySchema = z.object({
	name: z.string(),
	type: RepositoryTypeSchema,
	downloadUrl: z.string().optional(), // Required for tukui
	gitRemote: z.string().optional(), // Required for github
	branch: z.string().default("main"),
	folders: z.array(z.string()), // The folder names inside Interface/Addons
	installedVersion: z.string().nullable().default(null),
});

export const ConfigSchema = z.object({
	destDir: z
		.string()
		.describe("Path to World of Warcraft/_retail_/Interface/AddOns"),
	userAgent: z
		.string()
		.default(
			"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
		),
	repositories: z.array(RepositorySchema).default([]),
});

export type Repository = z.infer<typeof RepositorySchema>;
export type Config = z.infer<typeof ConfigSchema>;

// --- Configuration Manager ---

const PROJECT_NAME = "lemonup";

interface ConfigManagerOptions {
	cwd?: string; // Optional: Force a specific config directory (good for testing/dev)
}

export class ConfigManager {
	private store: Conf<Config>;

	constructor(options: ConfigManagerOptions = {}) {
		this.store = new Conf<Config>({
			projectName: PROJECT_NAME,
			schema: {
				destDir: { type: "string" },
				userAgent: { type: "string" },
				repositories: { type: "array", items: { type: "object" } }, // Conf schema is limited, we use Zod for real validation
			} as const,

			cwd: options.cwd || path.join(os.homedir(), ".config", "lemonup"),
			projectSuffix: "",
		});
	}

	public get(): Config {
		const raw = this.store.store;
		// Validate on read to ensure integrity
		const result = ConfigSchema.safeParse(raw);
		if (!result.success) {
			// If schema is invalid (e.g. first run or corrupted), return defaults or empty structure
			return {
				destDir: "NOT_CONFIGURED",
				userAgent: "DEFAULT_UA",
				repositories: [],
			} as unknown as Config;
		}
		return result.data;
	}

	public set<K extends keyof Config>(key: K, value: Config[K]) {
		this.store.set(key, value);
	}

	public updateRepository(repoName: string, updates: Partial<Repository>) {
		const current = this.get();
		const repos = [...current.repositories];
		const index = repos.findIndex((r) => r.name === repoName);

		if (index !== -1) {
			const updatedRepo = { ...repos[index], ...updates } as Repository;
			repos[index] = updatedRepo;
			this.store.set("repositories", repos);
		}
	}

	public get path(): string {
		return this.store.path;
	}

	public get hasConfigFile(): boolean {
		return fs.existsSync(this.store.path);
	}

	public createDefaultConfig(): void {
		const defaults: Config = {
			destDir: "NOT_CONFIGURED",
			userAgent:
				"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
			repositories: [],
		};
		this.store.set(defaults);
	}
}
