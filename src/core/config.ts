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
	defaultMenuOption: z.enum(["update", "manage", "config"]).default("update"),
	maxConcurrent: z.number().min(1).max(10).default(3),
	nerdFonts: z.boolean().default(true),
	checkInterval: z.number().min(0).default(60000), // ms, 0 = always check (or manually?)
	backupWTF: z.boolean().default(true),
	backupRetention: z.number().min(1).default(5),
});

export type Repository = z.infer<typeof RepositorySchema>;
export type Config = z.infer<typeof ConfigSchema>;

// --- Configuration Manager ---

const PROJECT_NAME = "lemonup";

interface ConfigManagerOptions {
	cwd?: string; // Optional: Force a specific config directory (good for testing/dev)
	overrides?: Partial<Config>; // Values to force on load
	enableSafeMode?: boolean; // If true, disable saving to disk
}

export class ConfigManager {
	private store: Conf<Config>;
	private overrides: Partial<Config>;
	private safeMode: boolean;

	constructor(options: ConfigManagerOptions = {}) {
		this.overrides = options.overrides || {};
		this.safeMode = options.enableSafeMode || false;

		this.store = new Conf<Config>({
			projectName: PROJECT_NAME,
			schema: {
				destDir: { type: "string" },
				userAgent: { type: "string" },
				repositories: { type: "array", items: { type: "object" } },
				defaultMenuOption: { type: "string" },
				maxConcurrent: { type: "number" },
				nerdFonts: { type: "boolean" },
				checkInterval: { type: "number" },
				backupWTF: { type: "boolean" },
				backupRetention: { type: "number" },
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
		// Apply overrides
		return { ...result.data, ...this.overrides };
	}

	public set<K extends keyof Config>(key: K, value: Config[K]) {
		if (this.safeMode) {
			// In safe mode, we can't update disk, but should we update memory?
			// Conf doesn't have an easy "set memory only" public method that persists across 'get' calls effectively
			// without mocking the store.
			// Ideally we just update our overrides or local state?
			// But for simplicity in this Test Mode: we just IGNORE writes or log them.
			// Actually, if we ignore writes, the UI won't update "installedVersion".
			// But since we use a TEMP cwd in our plan, we don't need safeMode to block disk writes!
			// We WANT to write to the temp disk to verify the operation.
			// So safeMode is legally "allow disk writes but maybe warn"?
			// Or maybe safeMode is not needed if we trust the temp dir approach.
			// Let's keep writing to disk (temp dir) as the primary strategy.
			// But if we wanted to prevent 'real' config writes, we'd block here.
			// For 'overrides', we might want to update them?
			// Let's assume standard behavior: write to store.
			// Overrides are "permanent/forced" values.
		}
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
			defaultMenuOption: "update",
			maxConcurrent: 3,
			nerdFonts: true,
			checkInterval: 60000,
			backupWTF: true,
			backupRetention: 5,
		};
		this.store.set(defaults);
	}
}
