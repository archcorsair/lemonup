import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { ScanCommand } from "@/core/commands/ScanCommand";
import type { Command, CommandContext } from "@/core/commands/types";
import { type ConfigManager, REPO_TYPE } from "@/core/config";
import type { DatabaseManager } from "@/core/db";
import * as Downloader from "@/core/downloader";
import { logger } from "@/core/logger";
import { isPathConfigured } from "@/core/paths";
import * as Wago from "@/core/wago";

export interface InstallWagoResult {
  success: boolean;
  installedAddons: string[];
  error?: string;
}

export class InstallWagoCommand implements Command<InstallWagoResult> {
  private installedFolders: string[] = [];

  constructor(
    private dbManager: DatabaseManager,
    private configManager: ConfigManager,
    private addonIdOrUrl: string,
    private stability: Wago.WagoStability = "stable",
  ) {}

  async execute(context: CommandContext): Promise<InstallWagoResult> {
    const config = this.configManager.get();

    if (!config.wagoApiKey) {
      return {
        success: false,
        installedAddons: [],
        error:
          "Wago API key not configured. Add your API key in Settings or set WAGO_API_KEY environment variable.",
      };
    }

    if (!isPathConfigured(config.destDir)) {
      return {
        success: false,
        installedAddons: [],
        error: "WoW Addon directory is not configured. Please go to Settings.",
      };
    }

    // Extract addon ID from URL if needed
    let addonId = this.addonIdOrUrl.trim();
    if (this.addonIdOrUrl.includes("wago.io")) {
      const parsed = Wago.getAddonIdFromUrl(this.addonIdOrUrl);
      if (!parsed || parsed.trim() === "") {
        return {
          success: false,
          installedAddons: [],
          error: "Could not parse addon ID from Wago URL",
        };
      }
      addonId = parsed;
    }

    if (!addonId) {
      return {
        success: false,
        installedAddons: [],
        error: "Addon ID is required",
      };
    }

    const tempDir = path.join(
      os.tmpdir(),
      `lemonup-wago-${crypto.randomUUID()}`,
    );

    try {
      logger.log("InstallWagoCommand", `Installing addon: ${addonId}`);
      context.emit("addon:install:start", addonId);

      // Fetch addon details
      const result = await Wago.getAddonDetails(addonId, config.wagoApiKey);
      if (!result.success) {
        const errorMsg =
          result.error === "not_found"
            ? `Addon "${addonId}" not found on Wago`
            : result.error === "no_api_key"
              ? "Wago API key required"
              : "Failed to fetch addon details from Wago";
        throw new Error(errorMsg);
      }

      const addon = result.addon;

      // Find best stability if requested one isn't available
      let effectiveStability = this.stability;
      if (!addon.releases[this.stability]) {
        const best = Wago.getBestAvailableStability(addon);
        if (!best) {
          throw new Error(
            `No releases available for addon "${addon.display_name}"`,
          );
        }
        effectiveStability = best;
        logger.log(
          "InstallWagoCommand",
          `Using ${effectiveStability} release (${this.stability} not available)`,
        );
      }

      // Get download URL
      const downloadUrl = Wago.getDownloadUrl(addon, effectiveStability);
      if (!downloadUrl) {
        throw new Error(`No download URL for ${effectiveStability} release`);
      }

      // Validate download URL is from Wago domain (security check)
      try {
        const urlObj = new URL(downloadUrl);
        if (!urlObj.hostname.endsWith("wago.io")) {
          throw new Error("Invalid download URL domain");
        }
      } catch (urlError) {
        if (
          urlError instanceof Error &&
          urlError.message === "Invalid download URL domain"
        ) {
          throw urlError;
        }
        throw new Error("Invalid download URL format");
      }

      context.emit("addon:install:downloading", addon.display_name);

      // Download and extract
      await fs.mkdir(tempDir, { recursive: true });
      const zipPath = path.join(tempDir, "addon.zip");

      // Download with auth header
      const downloadResponse = await fetch(downloadUrl, {
        headers: {
          Authorization: `Bearer ${config.wagoApiKey}`,
          Accept: "application/octet-stream",
        },
      });

      if (!downloadResponse.ok) {
        throw new Error(`Download failed: ${downloadResponse.status}`);
      }

      const zipData = await downloadResponse.arrayBuffer();
      await Bun.write(zipPath, zipData);
      context.emit("addon:install:extracting", addon.display_name);
      await Downloader.unzip(zipPath, tempDir);
      await fs.unlink(zipPath);

      // Scan for first-level folders containing .toc
      const tocGlob = new Bun.Glob("**/*.toc");
      const foundFolders = new Set<string>();

      for await (const file of tocGlob.scan({ cwd: tempDir })) {
        const dir = path.dirname(file);
        if (dir !== ".") {
          const firstLevel = dir.split("/")[0];
          if (firstLevel) {
            foundFolders.add(firstLevel);
          }
        }
      }

      const foldersToCopy = Array.from(foundFolders);
      const installedNames: string[] = [];

      context.emit("addon:install:copying", addon.display_name);

      // Copy folders to addons directory
      for (const folderName of foldersToCopy) {
        const source = path.join(tempDir, folderName);
        const dest = path.join(config.destDir, folderName);
        await fs.cp(source, dest, { recursive: true, force: true });
        installedNames.push(folderName);
        this.installedFolders.push(folderName);
      }

      if (installedNames.length === 0) {
        throw new Error("No addon folders found in download");
      }

      // Determine parent folder
      const parentFolder = this.determineParentFolder(
        installedNames,
        addon.display_name,
      );
      const ownedFolders = installedNames.filter((f) => f !== parentFolder);

      // Scan and register in database
      const scanCmd = new ScanCommand(this.dbManager, this.configManager, [
        parentFolder,
      ]);
      await scanCmd.execute(context);

      // Update addon record with Wago metadata
      const version = Wago.getVersion(addon, effectiveStability);
      const author = addon.owner ?? addon.authors?.[0] ?? "";
      this.dbManager.updateAddon(parentFolder, {
        url: `https://addons.wago.io/addons/${addonId}`,
        type: REPO_TYPE.WAGO,
        version: version || effectiveStability,
        author,
        last_updated: new Date().toISOString(),
        last_checked: new Date().toISOString(),
        remote_version: version || effectiveStability,
        ownedFolders,
      });

      // Remove stale DB records for owned folders
      for (const ownedFolder of ownedFolders) {
        this.dbManager.removeAddon(ownedFolder);
      }

      if (ownedFolders.length > 0) {
        context.emit("install:folder_ownership", parentFolder, ownedFolders);
      }

      context.emit("addon:install:complete", addon.display_name);
      return { success: true, installedAddons: installedNames };
    } catch (error) {
      logger.error("InstallWagoCommand", "Install failed", error);
      await this.undo(context);
      return {
        success: false,
        installedAddons: [],
        error: error instanceof Error ? error.message : String(error),
      };
    } finally {
      await fs.rm(tempDir, { recursive: true, force: true });
    }
  }

  async undo(_context: CommandContext): Promise<void> {
    logger.log("InstallWagoCommand", "Rolling back installation");
    const destDir = this.configManager.get().destDir;
    for (const folder of this.installedFolders) {
      try {
        await fs.rm(path.join(destDir, folder), {
          recursive: true,
          force: true,
        });
        this.dbManager.removeAddon(folder);
      } catch (err) {
        logger.error(
          "InstallWagoCommand",
          `Failed to remove ${folder} during undo`,
          err,
        );
      }
    }
  }

  /**
   * Determines which folder is the "parent" addon from a multi-folder install.
   * Uses heuristics based on naming patterns and display name.
   */
  private determineParentFolder(
    folders: string[],
    displayName: string,
  ): string {
    if (folders.length === 0) {
      throw new Error("No folders provided");
    }

    const [first] = folders;
    if (folders.length === 1 && first) {
      return first;
    }

    // Exact match with display name (case-insensitive, ignoring spaces)
    const normalizedDisplayName = displayName.replace(/\s+/g, "").toLowerCase();
    const exactMatch = folders.find(
      (f) =>
        f.toLowerCase() === normalizedDisplayName ||
        f.replace(/[-_]/g, "").toLowerCase() === normalizedDisplayName,
    );
    if (exactMatch) return exactMatch;

    // Shortest prefix that matches most folders
    const sorted = [...folders].sort((a, b) => a.length - b.length);
    const [shortest] = sorted;
    if (!shortest) {
      throw new Error("Unexpected empty sorted array");
    }
    const prefixCount = sorted.filter((n) => n.startsWith(shortest)).length;
    if (prefixCount / sorted.length >= 0.5) {
      return shortest;
    }

    return shortest;
  }
}
