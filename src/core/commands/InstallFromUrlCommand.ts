import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { ScanCommand } from "@/core/commands/ScanCommand";
import type { Command, CommandContext } from "@/core/commands/types";
import { type ConfigManager, REPO_TYPE } from "@/core/config";
import type { DatabaseManager } from "@/core/db";
import * as Downloader from "@/core/downloader";
import * as GitClient from "@/core/git";
import { logger } from "@/core/logger";
import { isPathConfigured } from "@/core/paths";
import * as WoWInterface from "@/core/wowinterface";

export interface InstallFromUrlResult {
  success: boolean;
  installedAddons: string[];
  error?: string;
}

export class InstallFromUrlCommand implements Command<InstallFromUrlResult> {
  private installedFolders: string[] = [];

  constructor(
    private dbManager: DatabaseManager,
    private configManager: ConfigManager,
    private url: string,
  ) {}

  async execute(context: CommandContext): Promise<InstallFromUrlResult> {
    const config = this.configManager.get();

    if (!isPathConfigured(config.destDir)) {
      return {
        success: false,
        installedAddons: [],
        error: "WoW Addon directory is not configured. Please go to Settings.",
      };
    }

    const tempDir = path.join(
      os.tmpdir(),
      `lemonup-install-${crypto.randomUUID()}`,
    );

    try {
      const parsed = new URL(this.url);
      const isGitHub = parsed.hostname.endsWith("github.com");
      const isWoWInterface = parsed.hostname.endsWith("wowinterface.com");
      const isCurseforge = parsed.hostname.endsWith("curseforge.com");
      const isWago = parsed.hostname.endsWith("wago.io");

      if (isCurseforge) {
        throw new Error(
          "Curseforge addons are not currently supported by LemonUp",
        );
      }

      if (isWago) {
        throw new Error("Wago addons will be supported soon!");
      }

      if (!isGitHub && !isWoWInterface) {
        throw new Error(
          "Only github.com and wowinterface.com URLs are supported",
        );
      }

      logger.log("InstallFromUrlCommand", `Installing from URL: ${this.url}`);
      context.emit("addon:install:start", this.url);

      let repoType: "github" | "wowinterface";
      let wowInterfaceDetails: WoWInterface.WoWInterfaceAddonDetails | null =
        null;

      if (isGitHub) {
        repoType = "github";
        context.emit("addon:install:downloading", this.url);
        if (!(await GitClient.clone(this.url, "main", tempDir))) {
          throw new Error("Git Clone failed");
        }
      } else {
        repoType = "wowinterface";
        const addonId = WoWInterface.getAddonIdFromUrl(this.url);
        if (!addonId) {
          throw new Error("Could not parse WoWInterface Addon ID from URL");
        }

        context.emit("addon:install:downloading", this.url);
        const details = await WoWInterface.getAddonDetails(addonId);
        if (!details) {
          throw new Error(
            "Failed to fetch addon details from WoWInterface API",
          );
        }
        wowInterfaceDetails = details;

        const zipPath = path.join(tempDir, "addon.zip");
        await fs.mkdir(tempDir, { recursive: true });

        if (
          !(await Downloader.download(wowInterfaceDetails.UIDownload, zipPath))
        ) {
          throw new Error("Failed to download addon zip");
        }

        await Downloader.unzip(zipPath, tempDir);
        // Remove zip after extraction so it doesn't get copied
        await fs.unlink(zipPath);
      }

      // Scan for first-level folders containing .toc (ignore embedded libs in subfolders)
      const tocGlob = new Bun.Glob("**/*.toc");
      const foundFolders = new Set<string>();

      for await (const file of tocGlob.scan({ cwd: tempDir })) {
        const dir = path.dirname(file);
        if (dir !== ".") {
          // Only add first-level folders (no "/" in path = top-level addon folder)
          const firstLevel = dir.split("/")[0];
          if (firstLevel) {
            foundFolders.add(firstLevel);
          }
        }
      }

      const foldersToCopy = Array.from(foundFolders);
      const rootTocs = Array.from(
        new Bun.Glob("*.toc").scanSync({ cwd: tempDir }),
      );
      const installedNames: string[] = [];

      context.emit("addon:install:copying", this.url);

      if (rootTocs.length > 0) {
        for (const tocFile of rootTocs) {
          const addonName = path.basename(tocFile, ".toc");
          const dest = path.join(config.destDir, addonName);
          await fs.cp(tempDir, dest, { recursive: true, force: true });
          installedNames.push(addonName);
          this.installedFolders.push(addonName);
        }
      }

      if (foldersToCopy.length > 0) {
        for (const folderName of foldersToCopy) {
          // folderName is already first-level only (e.g., "Clique", not "Clique/libs/...")
          const source = path.join(tempDir, folderName);
          const dest = path.join(config.destDir, folderName);
          await fs.cp(source, dest, { recursive: true, force: true });
          installedNames.push(folderName);
          this.installedFolders.push(folderName);
        }
      }

      if (installedNames.length === 0) {
        throw new Error("No addons found in repository/archive");
      }

      // Determine parent folder BEFORE scanning
      let targetName = "";
      if (repoType === "github") {
        const pathname = parsed.pathname.replace(/\.git$/, "");
        targetName = pathname.split("/").pop() || "";
      } else if (repoType === "wowinterface" && wowInterfaceDetails !== null) {
        targetName = wowInterfaceDetails.UIFileName.replace(/\.zip$/i, "");
      }

      const parentFolder = this.determineParentFolder(
        installedNames,
        targetName,
        repoType === "wowinterface" ? wowInterfaceDetails : null,
      );

      // Owned folders = all folders except parent
      const ownedFolders = installedNames.filter((f) => f !== parentFolder);

      // Scan ONLY the parent folder (not all folders)
      const scanCmd = new ScanCommand(this.dbManager, this.configManager, [
        parentFolder,
      ]);
      await scanCmd.execute(context);

      // Update parent addon with source info and owned folders
      if (repoType === "github") {
        const installedHash =
          (await GitClient.getCurrentCommit(tempDir)) || null;
        this.dbManager.updateAddon(parentFolder, {
          url: this.url,
          type: REPO_TYPE.GITHUB,
          git_commit: installedHash,
          last_updated: new Date().toISOString(),
          ownedFolders,
        });
      } else if (repoType === "wowinterface" && wowInterfaceDetails !== null) {
        this.dbManager.updateAddon(parentFolder, {
          url: this.url,
          type: REPO_TYPE.WOWINTERFACE,
          version: wowInterfaceDetails.UIVersion,
          author: wowInterfaceDetails.UIAuthorName,
          last_updated: new Date().toISOString(),
          ownedFolders,
        });
      }

      // Remove any stale DB records for owned folders (from previous scans)
      for (const ownedFolder of ownedFolders) {
        this.dbManager.removeAddon(ownedFolder);
      }

      // Emit ownership event if multi-folder
      if (ownedFolders.length > 0) {
        context.emit("install:folder_ownership", parentFolder, ownedFolders);
      }

      context.emit("addon:install:complete", this.url);
      return { success: true, installedAddons: installedNames };
    } catch (error) {
      logger.error("InstallFromUrlCommand", "Install failed", error);
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
    logger.log("InstallFromUrlCommand", "Rolling back installation");
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
          "InstallFromUrlCommand",
          `Failed to remove ${folder} during undo`,
          err,
        );
      }
    }
  }

  /**
   * Determines which folder is the "parent" addon from a multi-folder install.
   * Uses heuristics based on naming patterns and source metadata.
   */
  private determineParentFolder(
    folders: string[],
    targetName: string,
    wowInterfaceDetails: WoWInterface.WoWInterfaceAddonDetails | null,
  ): string {
    // Should never happen - caller ensures at least one folder
    if (folders.length === 0) {
      throw new Error("No folders provided to determineParentFolder");
    }

    const [first] = folders;
    if (folders.length === 1 && first) {
      return first;
    }

    // Exact match with target name (case-insensitive)
    const exactMatch = folders.find(
      (f) => f.toLowerCase() === targetName.toLowerCase(),
    );
    if (exactMatch) return exactMatch;

    // WoWInterface UIName match
    if (wowInterfaceDetails) {
      const uiNameMatch = folders.find(
        (f) => f.toLowerCase() === wowInterfaceDetails.UIName.toLowerCase(),
      );
      if (uiNameMatch) return uiNameMatch;
    }

    // Shortest prefix that matches most other folders
    const sorted = [...folders].sort((a, b) => a.length - b.length);
    const [shortest] = sorted;
    if (!shortest) {
      throw new Error("Unexpected empty sorted array");
    }
    const prefixCount = sorted.filter((n) => n.startsWith(shortest)).length;
    // If shortest is prefix for at least 50% of items, assume it's parent
    if (prefixCount / sorted.length >= 0.5) {
      return shortest;
    }

    // Target name similarity (substring match)
    if (targetName) {
      const candidates = folders.filter(
        (name) =>
          targetName.toLowerCase().includes(name.toLowerCase()) ||
          name.toLowerCase().includes(targetName.toLowerCase()),
      );
      const [firstCandidate] = candidates.sort((a, b) => a.length - b.length);
      if (firstCandidate) {
        return firstCandidate;
      }
    }

    // Fallback to shortest folder name
    return shortest;
  }
}
