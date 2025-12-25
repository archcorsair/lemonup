import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import Conf from "conf";
import { z } from "zod";
import { logger } from "./logger";
import { getDefaultWoWPath } from "./paths";

// --- Constants ---

export const REPO_TYPE = {
  GITHUB: "github",
  TUKUI: "tukui",
  WOWINTERFACE: "wowinterface",
} as const;

export type RepoType = (typeof REPO_TYPE)[keyof typeof REPO_TYPE];

// --- Zod Schemas ---

export const RepositoryTypeSchema = z.enum([
  REPO_TYPE.GITHUB,
  REPO_TYPE.TUKUI,
  REPO_TYPE.WOWINTERFACE,
]);

export const RepositorySchema = z.object({
  name: z.string(),
  type: RepositoryTypeSchema,
  downloadUrl: z.string().optional(),
  gitRemote: z.string().optional(),
  branch: z.string().default("main"),
  folders: z.array(z.string()),
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
  defaultMenuOption: z
    .enum(["update", "install", "manage", "config"])
    .default("update"),
  maxConcurrent: z.number().min(1).max(10).default(3),
  nerdFonts: z.boolean().default(true),
  checkInterval: z
    .number()
    .min(60000)
    .default(60000 * 5), // 5 minutes
  backupWTF: z.boolean().default(true),
  backupRetention: z.number().min(1).default(5),
  debug: z.boolean().default(false),
  migrated_to_db: z.boolean().optional().default(false),
  showLibs: z.boolean().default(false),
});

export type Repository = z.infer<typeof RepositorySchema>;
export type Config = z.infer<typeof ConfigSchema>;

// --- Configuration Manager ---

const PROJECT_NAME = "lemonup";

interface ConfigManagerOptions {
  cwd?: string;
  overrides?: Partial<Config>;
  enableSafeMode?: boolean;
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
        debug: { type: "boolean" },
        migrated_to_db: { type: "boolean" },
        showLibs: { type: "boolean" },
      } as const,

      cwd: options.cwd || path.join(os.homedir(), ".config", "lemonup"),
      projectSuffix: "",
    });
  }

  public get(): Config {
    const raw = this.store.store;
    const result = ConfigSchema.safeParse(raw);
    if (!result.success) {
      // If schema is invalid (first run or corrupted) return defaults or empty structure
      return {
        destDir: "NOT_CONFIGURED",
        userAgent: "DEFAULT_UA",
        repositories: [],
        defaultMenuOption: "update",
        maxConcurrent: 3,
        nerdFonts: true,
        checkInterval: 60000 * 5,
        backupWTF: true,
        backupRetention: 5,
        debug: false,
        migrated_to_db: false,
        showLibs: false,
        ...this.overrides,
      } as unknown as Config;
    }
    const config = { ...result.data, ...this.overrides };
    logger.setEnabled(config.debug || false);
    return config;
  }

  public set<K extends keyof Config>(key: K, value: Config[K]) {
    if (this.safeMode) {
      // TODO
      return;
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
      logger.log(
        "Config",
        `Updated ${repoName} to version ${updatedRepo.installedVersion}`,
      );
      console.log(
        `[Config] Updated ${repoName} to version ${updatedRepo.installedVersion}`,
      );
    } else {
      logger.error("Config", `Keep failed: Repo ${repoName} not found`);
      console.warn(`[Config] Keep failed: Repo ${repoName} not found`);
    }
  }

  public removeRepository(repoName: string) {
    const current = this.get();
    const repos = current.repositories.filter((r) => r.name !== repoName);
    if (repos.length !== current.repositories.length) {
      this.store.set("repositories", repos);
      logger.log("Config", `Removed repository: ${repoName}`);
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
      destDir: getDefaultWoWPath(),
      userAgent:
        "Mozilla/5.0 (SMART-REFRIGERATOR; Linux; Tizen 6.0) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/4.0 TV Safari/537.36",
      repositories: [],
      defaultMenuOption: "update",
      maxConcurrent: 3,
      nerdFonts: true,
      checkInterval: 60000 * 5,
      backupWTF: true,
      backupRetention: 5,
      debug: false,
      migrated_to_db: false,
      showLibs: false,
    };
    this.store.set(defaults);
  }
}
