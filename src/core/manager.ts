import { EventEmitter } from "node:events";
import path from "node:path";
import {
  InstallFromUrlCommand,
  type InstallFromUrlResult,
} from "./commands/InstallFromUrlCommand";
import { InstallTukUICommand } from "./commands/InstallTukUICommand";
import {
  InstallWagoCommand,
  type InstallWagoResult,
} from "./commands/InstallWagoCommand";
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
import * as TukUI from "./tukui";
import * as Wago from "./wago";
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
  private autoCheckTimer: NodeJS.Timeout | null = null;
  private isAutoChecking = false;

  constructor(configManager?: ConfigManager) {
    super();
    this.configManager = configManager || new ConfigManager();
    const configDir = path.dirname(this.configManager.path);
    this.dbManager = new DatabaseManager(configDir);
  }

  public startAutoCheckLoop() {
    this.stopAutoCheckLoop();

    const config = this.configManager.get();
    if (!config.autoCheckEnabled) return;

    const scheduleNext = (delay: number) => {
      this.autoCheckTimer = setTimeout(() => {
        this.runGlobalCheck().finally(() => {
          // Re-read config in case interval changed
          const currentConfig = this.configManager.get();
          if (currentConfig.autoCheckEnabled) {
            scheduleNext(currentConfig.autoCheckInterval);
          }
        });
      }, delay);
    };

    const lastCheck = config.lastGlobalCheck || 0;
    const now = Date.now();
    const interval = config.autoCheckInterval;
    const timeSinceLast = now - lastCheck;
    const MIN_STARTUP_DELAY = 5000;

    let initialDelay = MIN_STARTUP_DELAY;

    if (lastCheck > 0 && timeSinceLast < interval) {
      const remaining = interval - timeSinceLast;
      initialDelay = Math.max(MIN_STARTUP_DELAY, remaining);
    }

    scheduleNext(initialDelay);
  }

  public stopAutoCheckLoop() {
    if (this.autoCheckTimer) {
      clearTimeout(this.autoCheckTimer);
      this.autoCheckTimer = null;
    }
  }

  private async runGlobalCheck() {
    if (this.isAutoChecking) return;
    this.isAutoChecking = true;

    try {
      const addons = this.dbManager.getAll().filter((a) => a.type !== "manual");

      if (addons.length === 0) return;

      this.emit("autocheck:start", addons.length);
      let updatesFound = 0;

      for (let i = 0; i < addons.length; i++) {
        const addon = addons[i];
        if (!addon) continue;

        try {
          const result = await this.checkUpdate(addon);
          if (result.updateAvailable) {
            updatesFound++;
          }
        } catch (error) {
          this.emit(
            "error",
            `Auto-check failed for ${addon.name}`,
            String(error),
          );
        }

        this.emit("autocheck:progress", i + 1, addons.length, addon.name);
      }

      this.configManager.set("lastGlobalCheck", Date.now());
      this.emit("autocheck:complete", updatesFound);
    } catch (error) {
      this.emit("error", "Global check failed", String(error));
    } finally {
      this.isAutoChecking = false;
    }
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
    this.stopAutoCheckLoop();
    this.dbManager.close();
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

  public getCachedUpdateStatus(addon: AddonRecord): {
    updateAvailable: boolean;
    remoteVersion: string;
    cached: boolean;
  } | null {
    const config = this.configManager.get();
    const lastChecked = addon.last_checked
      ? new Date(addon.last_checked).getTime()
      : 0;
    const isStale = Date.now() - lastChecked > config.checkInterval;

    if (!isStale && addon.remote_version) {
      const remoteVersion = addon.remote_version;
      let updateAvailable = false;

      if (addon.type === "github") {
        const localHash = addon.git_commit || addon.version;
        if (!localHash) {
          updateAvailable = true;
        } else {
          updateAvailable = localHash !== remoteVersion;
          if (
            remoteVersion.startsWith(localHash) ||
            localHash.startsWith(remoteVersion)
          ) {
            updateAvailable = false;
          }
        }
      } else {
        // Simple version comparison for others
        updateAvailable = addon.version !== remoteVersion;
      }

      return {
        updateAvailable,
        remoteVersion,
        cached: true,
      };
    }
    return null;
  }

  public async checkUpdate(
    addon: AddonRecord,
    force = false,
  ): Promise<{
    updateAvailable: boolean;
    remoteVersion: string;
    error?: string;
    cached?: boolean;
  }> {
    // TODO: Check if addon is owned by another via getOwnerOf

    if (!addon.url) {
      return {
        updateAvailable: false,
        remoteVersion: "",
        error: "Missing URL",
      };
    }

    // Use cached result if available and not stale
    if (!force) {
      const cached = this.getCachedUpdateStatus(addon);
      if (cached) {
        return cached;
      }
    }

    // Perform live check
    const config = this.configManager.get();
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

      // Update DB
      this.dbManager.updateAddon(addon.folder, {
        last_checked: new Date().toISOString(),
        remote_version: remoteHash,
      });

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
          // Update DB
          this.dbManager.updateAddon(addon.folder, {
            last_checked: new Date().toISOString(),
            remote_version: details.version,
          });

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

      const result = await WoWInterface.getAddonDetails(addonId);
      if (!result.success) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error:
            result.error === "not_found"
              ? "Addon not found on WoWInterface"
              : "Failed to fetch WoWInterface details",
        };
      }

      // Update DB
      this.dbManager.updateAddon(addon.folder, {
        last_checked: new Date().toISOString(),
        remote_version: result.details.UIVersion,
      });

      // Simple string comparison for now, assuming UIVersion changes on update
      const isUpdate = result.details.UIVersion !== addon.version;
      return {
        updateAvailable: isUpdate,
        remoteVersion: result.details.UIVersion,
      };
    }

    if (addon.type === "wago") {
      const addonId = Wago.getAddonIdFromUrl(addon.url);
      if (!addonId) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "Invalid Wago URL",
        };
      }

      const apiKey = config.wagoApiKey;
      if (!apiKey) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "Wago API key not configured",
        };
      }

      const result = await Wago.getAddonDetails(addonId, apiKey);
      if (!result.success) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error:
            result.error === "not_found"
              ? "Addon not found on Wago"
              : result.error === "no_api_key"
                ? "Wago API key missing"
                : "Failed to fetch Wago details",
        };
      }

      // Determine remote version (prefer stable, fallback to best)
      let remoteVersion: string | null | undefined =
        Wago.getVersion(result.addon, "stable") ??
        result.addon.releases.stable?.label;

      if (!remoteVersion) {
        const best = Wago.getBestAvailableStability(result.addon);
        if (best) {
          remoteVersion = Wago.getVersion(result.addon, best);
        }
      }

      if (!remoteVersion) {
        return {
          updateAvailable: false,
          remoteVersion: "",
          error: "No release version found",
        };
      }

      // Update DB
      this.dbManager.updateAddon(addon.folder, {
        last_checked: new Date().toISOString(),
        remote_version: remoteVersion,
      });

      const isUpdate = remoteVersion !== addon.version;
      return {
        updateAvailable: isUpdate,
        remoteVersion: remoteVersion,
      };
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

  public async installWago(
    addonIdOrUrl: string,
    stability: Wago.WagoStability = "stable",
  ): Promise<InstallWagoResult> {
    const command = new InstallWagoCommand(
      this.dbManager,
      this.configManager,
      addonIdOrUrl,
      stability,
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
