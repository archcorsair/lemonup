import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import Conf from "conf";
import { z } from "zod";
import { logger } from "./logger";

// --- Constants ---

export const REPO_TYPE = {
  GITHUB: "github",
  TUKUI: "tukui",
  WOWINTERFACE: "wowinterface",
  WAGO: "wago",
} as const;

export type RepoType = (typeof REPO_TYPE)[keyof typeof REPO_TYPE];

// --- Zod Schemas ---

// Legacy schema - kept for backwards compatibility with old config files
const RepositorySchema = z.object({
  name: z.string(),
  type: z.enum(["github", "tukui", "wowinterface"]),
  downloadUrl: z.string().optional(),
  gitRemote: z.string().optional(),
  branch: z.string().default("main"),
  folders: z.array(z.string()),
  installedVersion: z.string().nullable().default(null),
});

export const ConfigSchema = z.object({
  destDir: z
    .string()
    .describe("Path to World of Warcraft/_retail_/Interface/AddOns")
    .default("NOT_CONFIGURED"),
  userAgent: z
    .string()
    .default(
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
    ),
  // Legacy field - kept for backwards compatibility, no longer written to
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
  autoCheckEnabled: z.boolean().default(true),
  autoCheckInterval: z
    .number()
    .min(60 * 1000) // 1 minute (dev mode), UI enforces 30min for normal use
    .max(4 * 60 * 60 * 1000) // 4 hours
    .default(60 * 60 * 1000), // 1 hour
  backupWTF: z.boolean().default(true),
  backupRetention: z.number().min(1).default(5),
  debug: z.boolean().default(false),
  showLibs: z.boolean().default(false),
  theme: z.enum(["dark", "light"]).default("dark"),
  terminalProgress: z.boolean().default(true),
  wagoApiKey: z
    .string()
    .describe("Wago.io API key for addon downloads")
    .default(""),
  lastGlobalCheck: z.number().default(0),
});

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
        autoCheckEnabled: { type: "boolean" },
        autoCheckInterval: { type: "number" },
        backupWTF: { type: "boolean" },
        backupRetention: { type: "number" },
        debug: { type: "boolean" },
        showLibs: { type: "boolean" },
        theme: { type: "string" },
        terminalProgress: { type: "boolean" },
        wagoApiKey: { type: "string" },
        lastGlobalCheck: { type: "number" },
      } as const,

      cwd: options.cwd || path.join(os.homedir(), ".config", "lemonup"),
      projectSuffix: "",
    });
  }

  public get(): Config {
    const raw = this.store.store;
    const result = ConfigSchema.safeParse(raw);
    if (!result.success) {
      // Log validation error only if debug is enabled, or warn
      if (this.store.store.debug) {
        logger.error(
          "Config",
          `Validation failed: ${JSON.stringify(result.error)}`,
        );
      }

      // If schema is invalid (first run or corrupted) return defaults or empty structure
      // Try to preserve raw values that might be valid (like theme)
      const fallback = {
        destDir: "NOT_CONFIGURED",
        userAgent: "DEFAULT_UA",
        repositories: [],
        defaultMenuOption: "update",
        maxConcurrent: 3,
        nerdFonts: true,
        checkInterval: 60000 * 5,
        autoCheckEnabled: true,
        autoCheckInterval: 60 * 60 * 1000,
        backupWTF: true,
        backupRetention: 5,
        debug: false,
        showLibs: false,
        theme: "dark",
        terminalProgress: true,
        wagoApiKey: "",
        ...(raw as object),
        ...this.overrides,
      } as unknown as Config;

      // Apply env var fallback for wagoApiKey in fallback path too
      if (!fallback.wagoApiKey) {
        const envKey = process.env.WAGO_API_KEY;
        if (envKey) {
          fallback.wagoApiKey = envKey;
        }
      }

      return fallback;
    }
    const config = { ...result.data, ...this.overrides };

    // Only use env var if stored value is empty
    if (!config.wagoApiKey) {
      const envKey = process.env.WAGO_API_KEY;
      if (envKey) {
        config.wagoApiKey = envKey;
      }
    }

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

  public get path(): string {
    return this.store.path;
  }

  public get hasConfigFile(): boolean {
    return fs.existsSync(this.store.path);
  }

  public createDefaultConfig(): void {
    const defaults: Config = {
      destDir: "NOT_CONFIGURED", // Will be set by wizard or user
      userAgent:
        "Mozilla/5.0 (SMART-REFRIGERATOR; Linux; Tizen 6.0) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/4.0 TV Safari/537.36",
      repositories: [],
      defaultMenuOption: "update",
      maxConcurrent: 3,
      nerdFonts: true,
      checkInterval: 60000 * 5,
      autoCheckEnabled: false,
      autoCheckInterval: 60 * 60 * 1000,
      backupWTF: true,
      backupRetention: 5,
      debug: false,
      showLibs: false,
      theme: "dark",
      terminalProgress: true,
      wagoApiKey: "",
      lastGlobalCheck: 0,
    };
    this.store.set(defaults);
  }
}
