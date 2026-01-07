import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "@/core/config";
import type { AddonRecord, DatabaseManager } from "@/core/db";
import * as Downloader from "@/core/downloader";
import * as GitClient from "@/core/git";
import { logger } from "@/core/logger";
import * as TukUI from "@/core/tukui";
import * as Wago from "@/core/wago";
import * as WoWInterface from "@/core/wowinterface";
import { ScanCommand } from "./ScanCommand";
import type { Command, CommandContext } from "./types";

export interface UpdateAddonResult {
  repoName: string;
  success: boolean;
  updated: boolean;
  message?: string;
  error?: string;
}

export class UpdateAddonCommand implements Command<UpdateAddonResult> {
  private backupPaths: Map<string, string> = new Map();
  private previousRecord: AddonRecord | null = null;

  constructor(
    private dbManager: DatabaseManager,
    private configManager: ConfigManager,
    private addon: AddonRecord,
    private force = false,
  ) {}

  async execute(context: CommandContext): Promise<UpdateAddonResult> {
    const { name, folder } = this.addon;

    // Check if this folder is owned by another addon
    const owner = this.dbManager.getOwnerOf(folder);
    if (owner) {
      return {
        repoName: name,
        success: false,
        updated: false,
        message: `Managed by ${owner.folder} - update that addon instead`,
      };
    }

    context.emit("addon:update-check:start", folder);

    this.previousRecord = { ...this.addon };

    let remoteVersion = "unknown";
    let updateAvailable = true;
    let wowInterfaceDetails: WoWInterface.WoWInterfaceAddonDetails | null =
      null;
    let tukuiDetails: TukUI.TukUIAddon | null = null;
    let wagoDetails: Wago.WagoAddonSummary | null = null;

    if (this.addon.type === "github") {
      try {
        const branch = "main";
        const remoteHash = await GitClient.getRemoteCommit(
          this.addon.url || "",
          branch,
        );
        if (!remoteHash) throw new Error("Failed to get remote hash");

        remoteVersion = remoteHash;
        const localHash = this.addon.git_commit || this.addon.version;

        // short vs full hash
        if (
          localHash &&
          (remoteHash.startsWith(localHash) || localHash.startsWith(remoteHash))
        ) {
          updateAvailable = false;
        } else {
          updateAvailable = localHash !== remoteHash;
        }
      } catch (err) {
        return {
          repoName: name,
          success: false,
          updated: false,
          error: String(err),
        };
      }
    } else if (this.addon.type === "wowinterface") {
      try {
        const addonId = WoWInterface.getAddonIdFromUrl(this.addon.url || "");
        if (!addonId) throw new Error("Invalid WoWInterface URL");

        const result = await WoWInterface.getAddonDetails(addonId);
        if (!result.success) {
          throw new Error(
            result.error === "not_found"
              ? "Addon not found on WoWInterface"
              : "Failed to fetch details from WoWInterface",
          );
        }
        wowInterfaceDetails = result.details;

        remoteVersion = wowInterfaceDetails.UIVersion;
        updateAvailable = remoteVersion !== this.addon.version;
      } catch (err) {
        return {
          repoName: name,
          success: false,
          updated: false,
          error: String(err),
        };
      }
    } else if (this.addon.type === "tukui") {
      try {
        tukuiDetails = await TukUI.getAddonDetails(name);

        if (tukuiDetails) {
          remoteVersion = tukuiDetails.version;
          updateAvailable = remoteVersion !== this.addon.version;
        } else {
          throw new Error(`Could not find addon details for ${name} on TukUI`);
        }
      } catch (err) {
        return {
          repoName: name,
          success: false,
          updated: false,
          error: String(err),
        };
      }
    } else if (this.addon.type === "wago") {
      try {
        const config = this.configManager.get();
        if (!config.wagoApiKey) {
          throw new Error("Wago API key not configured");
        }

        const addonId = Wago.getAddonIdFromUrl(this.addon.url || "");
        if (!addonId) {
          throw new Error("Invalid Wago URL");
        }

        const result = await Wago.getAddonDetails(addonId, config.wagoApiKey);
        if (!result.success) {
          throw new Error(
            result.error === "not_found"
              ? "Addon not found on Wago"
              : "Failed to fetch details from Wago",
          );
        }
        wagoDetails = result.addon;

        const stability = Wago.getBestAvailableStability(wagoDetails);
        if (stability) {
          remoteVersion = Wago.getVersion(wagoDetails, stability) || "unknown";
          updateAvailable = remoteVersion !== this.addon.version;
        } else {
          throw new Error("No releases available for addon");
        }
      } catch (err) {
        return {
          repoName: name,
          success: false,
          updated: false,
          error: String(err),
        };
      }
    }

    context.emit(
      "addon:update-check:complete",
      folder,
      updateAvailable,
      remoteVersion,
    );

    if (!this.force && !updateAvailable) {
      return {
        repoName: name,
        success: true,
        updated: false,
        message: "Up to date",
      };
    }

    context.emit("addon:install:start", folder);
    const tempDir = path.join(os.tmpdir(), "lemonup-updates", name);
    await fs.rm(tempDir, { recursive: true, force: true });
    await fs.mkdir(tempDir, { recursive: true });

    try {
      let extractRoot = tempDir;

      if (this.addon.type === "tukui" && tukuiDetails) {
        context.emit("addon:install:downloading", folder);
        const zipPath = path.join(tempDir, "addon.zip");

        const downloadUrl = tukuiDetails.url;

        if (!(await Downloader.download(downloadUrl, zipPath))) {
          throw new Error("Download failed");
        }

        context.emit("addon:install:extracting", folder);
        const extractPath = path.join(tempDir, "extract");
        await fs.mkdir(extractPath, { recursive: true });
        if (!(await Downloader.unzip(zipPath, extractPath))) {
          throw new Error("Unzip failed");
        }
        extractRoot = extractPath;
      } else if (
        this.addon.type === "wowinterface" &&
        wowInterfaceDetails !== null
      ) {
        context.emit("addon:install:downloading", folder);
        const zipPath = path.join(tempDir, "addon.zip");
        if (
          !(await Downloader.download(wowInterfaceDetails.UIDownload, zipPath))
        ) {
          throw new Error("Download failed");
        }

        context.emit("addon:install:extracting", folder);
        const extractPath = path.join(tempDir, "extract");
        await fs.mkdir(extractPath, { recursive: true });
        if (!(await Downloader.unzip(zipPath, extractPath))) {
          throw new Error("Unzip failed");
        }
        extractRoot = extractPath;
      } else if (this.addon.type === "github") {
        context.emit("addon:install:downloading", folder);
        if (!(await GitClient.clone(this.addon.url || "", "main", tempDir))) {
          throw new Error("Git Clone failed");
        }
        extractRoot = tempDir;
      } else if (this.addon.type === "wago" && wagoDetails !== null) {
        const config = this.configManager.get();
        const stability = Wago.getBestAvailableStability(wagoDetails);
        const downloadUrl = stability
          ? Wago.getDownloadUrl(wagoDetails, stability)
          : null;

        if (!downloadUrl) {
          throw new Error("No download URL available");
        }

        // Validate download URL domain (security check)
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

        context.emit("addon:install:downloading", folder);
        const zipPath = path.join(tempDir, "addon.zip");

        const downloadResponse = await fetch(downloadUrl, {
          headers: {
            Authorization: `Bearer ${config.wagoApiKey}`,
            Accept: "application/octet-stream",
          },
        });

        if (!downloadResponse.ok) {
          throw new Error("Download failed");
        }

        const zipData = await downloadResponse.arrayBuffer();
        await Bun.write(zipPath, zipData);

        context.emit("addon:install:extracting", folder);
        const extractPath = path.join(tempDir, "extract");
        await fs.mkdir(extractPath, { recursive: true });
        if (!(await Downloader.unzip(zipPath, extractPath))) {
          throw new Error("Unzip failed");
        }
        extractRoot = extractPath;
      }

      // Scan for all valid addon folders in the extracted content
      const tocGlob = new Bun.Glob("**/*.toc");
      const foundFolders = new Set<string>();

      for await (const file of tocGlob.scan({ cwd: extractRoot })) {
        const dir = path.dirname(file);
        if (dir !== ".") {
          const parts = dir.split(path.sep);
          if (parts.length > 0 && parts[0]) {
            foundFolders.add(parts[0]);
          }
        } else {
          // FIXME:
          // TOC in root? Should not happen if we clone/extract correctly,
          // unless zip has no folders (bad zip).
        }
      }

      // Also check root for TOCs (some zips are loose files)
      const rootTocs = Array.from(
        new Bun.Glob("*.toc").scanSync({ cwd: extractRoot }),
      );
      if (rootTocs.length > 0) {
        // FIXME: What should we do if the zip is loose files?
      }

      const foldersToInstall = Array.from(foundFolders);
      if (foldersToInstall.length === 0 && rootTocs.length > 0) {
        // Single loose addon
        await this.backupAndInstall(context, extractRoot, folder);
        foldersToInstall.push(folder);
      } else {
        for (const f of foldersToInstall) {
          const source = path.join(extractRoot, f);
          await this.backupAndInstall(context, source, f);
        }
      }

      // Ensure all installed folders are in the DB
      const scanCmd = new ScanCommand(
        this.dbManager,
        this.configManager,
        foldersToInstall,
      );
      await scanCmd.execute(context);

      const isGitHash = remoteVersion.match(/^[a-f0-9]{40}$/);
      const newVersion = isGitHash
        ? remoteVersion.substring(0, 7)
        : remoteVersion;
      const newCommit = isGitHash ? remoteVersion : null;

      // Update the main addon record, preserving ownedFolders
      const ownedFolders = this.addon.ownedFolders || [];
      this.dbManager.updateAddon(this.addon.folder, {
        version: newVersion,
        git_commit: newCommit,
        last_updated: new Date().toISOString(),
        ownedFolders, // Preserve existing ownedFolders
      });

      // For multi-folder addons, remove any subfolder records (they're owned)
      for (const ownedFolder of ownedFolders) {
        this.dbManager.removeAddon(ownedFolder);
      }

      context.emit("addon:install:complete", folder);
      return {
        repoName: name,
        success: true,
        updated: true,
        message: `Updated to ${newVersion}`,
      };
    } catch (err) {
      logger.error("UpdateAddonCommand", `Error updating ${name}`, err);
      await this.undo(context);
      return {
        repoName: name,
        success: false,
        updated: false,
        error: err instanceof Error ? err.message : String(err),
      };
    } finally {
      await fs.rm(tempDir, { recursive: true, force: true });
    }
  }

  private async backupAndInstall(
    context: CommandContext,
    sourcePath: string,
    folder: string,
  ) {
    const destDir = this.configManager.get().destDir;
    const destPath = path.join(destDir, folder);

    try {
      await fs.access(destPath);
      const backupBase = path.join(os.tmpdir(), "lemonup-backups");
      await fs.mkdir(backupBase, { recursive: true });
      const backupPath = path.join(backupBase, `${folder}-${Date.now()}`);
      await fs.cp(destPath, backupPath, { recursive: true });
      this.backupPaths.set(folder, backupPath);
    } catch {
      // TODO: Send errors to logger
    }

    context.emit("addon:install:copying", folder);
    await fs.cp(sourcePath, destPath, { recursive: true, force: true });
  }
  async undo(_context: CommandContext): Promise<void> {
    logger.log(
      "UpdateAddonCommand",
      `Rolling back update for ${this.addon.name}`,
    );

    const destDir = this.configManager.get().destDir;

    for (const [folder, backupPath] of this.backupPaths) {
      const destPath = path.join(destDir, folder);
      try {
        await fs.rm(destPath, { recursive: true, force: true });
        await fs.cp(backupPath, destPath, { recursive: true });
      } catch (err) {
        logger.error(
          "UpdateAddonCommand",
          `Failed to restore ${folder} during undo`,
          err,
        );
      }
    }

    if (this.previousRecord) {
      this.dbManager.updateAddon(this.addon.folder, {
        version: this.previousRecord.version,
        last_updated:
          this.previousRecord.last_updated || new Date().toISOString(),
      });
    }
  }
}
