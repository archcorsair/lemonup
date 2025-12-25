import { EventEmitter } from "node:events";
import path from "node:path";
import {
  InstallFromUrlCommand,
  type InstallFromUrlResult,
} from "./commands/InstallFromUrlCommand";
import { InstallTukUICommand } from "./commands/InstallTukUICommand";
import {
  RemoveAddonCommand,
  type RemoveAddonResult,
} from "./commands/RemoveAddonCommand";
import { ScanCommand } from "./commands/ScanCommand";
import type { Command, CommandContext } from "./commands/types";
import {
  UpdateAddonCommand,
  type UpdateAddonResult,
} from "./commands/UpdateAddonCommand";
import { type Config, ConfigManager } from "./config";
import { type AddonRecord, DatabaseManager } from "./db";
import type { AddonManagerEvents } from "./events";
import * as GitClient from "./git";
import { logger } from "./logger";
import * as TukUI from "./tukui";
import * as WoWInterface from "./wowinterface";

export interface UpdateResult {
  repoName: string;
  success: boolean;
  updated: boolean;
  message?: string;
  error?: string;
}

export interface AddonManager {
  on<K extends keyof AddonManagerEvents>(
    event: K,
    listener: (...args: AddonManagerEvents[K]) => void,
  ): this;
  off<K extends keyof AddonManagerEvents>(
    event: K,
    listener: (...args: AddonManagerEvents[K]) => void,
  ): this;
  emit<K extends keyof AddonManagerEvents>(
    event: K,
    ...args: AddonManagerEvents[K]
  ): boolean;
}

// biome-ignore lint/suspicious/noUnsafeDeclarationMerging: Standard pattern for typed EventEmitter
export class AddonManager extends EventEmitter {
  private configManager: ConfigManager;
  private dbManager: DatabaseManager;

  constructor(configManager?: ConfigManager) {
    super();
    this.configManager = configManager || new ConfigManager();
    const configDir = path.dirname(this.configManager.path);
    this.dbManager = new DatabaseManager(configDir);

    this.migrateConfig();
  }

  private async executeCommand<T>(command: Command<T>): Promise<T> {
    const context: CommandContext = {
      emit: (event, ...args) => this.emit(event, ...args),
    };
    return await command.execute(context);
  }

  public getConfig(): Config {
    return this.configManager.get();
  }

  public setConfigValue<K extends keyof Config>(key: K, value: Config[K]) {
    this.configManager.set(key, value);
  }

  public close() {
    this.dbManager.close();
  }

  private migrateConfig() {
    const config = this.configManager.get();

    if (config.migrated_to_db) {
      return;
    }

    if (Array.isArray(config.repositories) && config.repositories.length > 0) {
      logger.log(
        "Manager",
        "Migrating repositories from config to database...",
      );
      for (const repo of config.repositories) {
        const folder = repo.folders[0];
        if (!folder) continue;

        if (this.dbManager.getByFolder(folder)) continue;

        this.dbManager.addAddon({
          name: repo.name,
          folder: folder,
          version: repo.installedVersion,
          git_commit: null,
          author: null,
          interface: null,
          url: repo.gitRemote || repo.downloadUrl || null,
          type: repo.type as "github" | "tukui",
          ownedFolders: [],
          kind: "addon",
          kindOverride: false,
          flavor: "retail",
          requiredDeps: [],
          optionalDeps: [],
          embeddedLibs: [],
          install_date: new Date().toISOString(),
          last_updated: new Date().toISOString(),
        });
        logger.log("Manager", `Migrated ${repo.name}`);
      }
    }

    this.configManager.set("migrated_to_db", true);
  }

  public async updateAll(force = false): Promise<UpdateAddonResult[]> {
    const addons = this.dbManager.getAll();
    const results: UpdateAddonResult[] = [];

    for (const addon of addons) {
      if (addon.type === "manual") continue;

      try {
        const result = await this.updateAddon(addon, force);
        results.push(result);
      } catch (error) {
        results.push({
          repoName: addon.name,
          success: false,
          updated: false,
          error: String(error),
        });
      }
    }

    return results;
  }

  public async checkUpdate(addon: AddonRecord): Promise<{
    updateAvailable: boolean;
    remoteVersion: string;
    error?: string;
  }> {
    // TODO: Check if addon is owned by another via getOwnerOf

    if (!addon.url) {
      return {
        updateAvailable: false,
        remoteVersion: "",
        error: "Missing URL",
      };
    }

    if (addon.type === "github") {
      const branch = "main";
      const remoteHash = await GitClient.getRemoteCommit(addon.url, branch);
      if (!remoteHash) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "Failed to get remote hash",
        };
      }

      // Compare with stored git_commit if available, otherwise fallback to version (legacy behavior)
      const localHash = addon.git_commit || addon.version;
      let isUpdate = localHash !== remoteHash;

      // Handle short vs full hash comparison
      if (localHash && remoteHash) {
        if (
          remoteHash.startsWith(localHash) ||
          localHash.startsWith(remoteHash)
        ) {
          isUpdate = false;
        }
      }

      return { updateAvailable: isUpdate, remoteVersion: remoteHash };
    }

    if (addon.type === "tukui") {
      try {
        const details = await TukUI.getAddonDetails(addon.name);
        if (details) {
          const isUpdate = details.version !== addon.version;
          return {
            updateAvailable: isUpdate,
            remoteVersion: details.version,
          };
        }
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "Addon details not found",
        };
      } catch (e) {
        return { updateAvailable: false, remoteVersion: "", error: String(e) };
      }
    }

    if (addon.type === "wowinterface") {
      const addonId = WoWInterface.getAddonIdFromUrl(addon.url);
      if (!addonId) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "Invalid WoWInterface URL",
        };
      }

      const details = await WoWInterface.getAddonDetails(addonId);
      if (!details) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "Failed to fetch WoWInterface details",
        };
      }

      // Simple string comparison for now, assuming UIVersion changes on update
      const isUpdate = details.UIVersion !== addon.version;
      return { updateAvailable: isUpdate, remoteVersion: details.UIVersion };
    }

    return { updateAvailable: false, remoteVersion: "" };
  }

  public async updateAddon(
    addon: AddonRecord,
    force: boolean,
  ): Promise<UpdateAddonResult> {
    const command = new UpdateAddonCommand(
      this.dbManager,
      this.configManager,
      addon,
      force,
    );
    return await this.executeCommand(command);
  }

  public async installFromUrl(url: string): Promise<InstallFromUrlResult> {
    const command = new InstallFromUrlCommand(
      this.dbManager,
      this.configManager,
      url,
    );
    return await this.executeCommand(command);
  }

  public async installTukUI(
    url: string,
    addonFolder: string,
    subFolders: string[] = [],
  ): Promise<boolean> {
    const command = new InstallTukUICommand(
      this.dbManager,
      this.configManager,
      url,
      addonFolder,
      subFolders,
    );
    return await this.executeCommand(command);
  }

  public getAllAddons() {
    return this.dbManager.getAll();
  }

  public getAddon(folder: string) {
    return this.dbManager.getByFolder(folder);
  }

  public async scanInstalledAddons(
    specificFolders?: string[],
  ): Promise<number> {
    const command = new ScanCommand(
      this.dbManager,
      this.configManager,
      specificFolders,
    );
    return await this.executeCommand(command);
  }

  public updateAddonMetadata(folder: string, metadata: Partial<AddonRecord>) {
    this.dbManager.updateAddon(folder, metadata);
  }

  public async removeAddon(
    folder: string,
    force = false,
  ): Promise<RemoveAddonResult> {
    const command = new RemoveAddonCommand(
      this.dbManager,
      this.configManager,
      folder,
      force,
    );
    return await this.executeCommand(command);
  }

  public isAlreadyInstalled(urlOrFolder: string): boolean {
    const addons = this.dbManager.getAll();
    const clean = (u: string) =>
      u
        .replace(/\/$/, "")
        .replace(/\.git$/, "")
        .toLowerCase();

    const target = clean(urlOrFolder);

    // Check by folder name or URL
    return addons.some(
      (a) => clean(a.folder) === target || (a.url && clean(a.url) === target),
    );
  }
}
