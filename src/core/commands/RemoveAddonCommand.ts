import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "@/core/config";
import type { AddonRecord, DatabaseManager } from "@/core/db";
import { logger } from "@/core/logger";
import type { Command, CommandContext } from "./types";

export interface RemoveAddonResult {
  success: boolean;
  removedFolders: string[];
  blocked?: { reason: string; dependents: string[] };
}

export class RemoveAddonCommand implements Command<RemoveAddonResult> {
  private addonRecord: AddonRecord | null = null;
  private backupPaths: Map<string, string> = new Map();

  constructor(
    private dbManager: DatabaseManager,
    private configManager: ConfigManager,
    private folder: string,
    private force = false,
  ) {}

  async execute(context: CommandContext): Promise<RemoveAddonResult> {
    context.emit("addon:remove:start", this.folder);
    this.addonRecord = this.dbManager.getByFolder(this.folder);

    if (!this.addonRecord) {
      logger.error("Manager", `Addon not found in DB: ${this.folder}`);
      return { success: false, removedFolders: [] };
    }

    // Check if this folder is owned by another addon
    const owner = this.dbManager.getOwnerOf(this.folder);
    if (owner) {
      return {
        success: false,
        removedFolders: [],
        blocked: {
          reason: `This folder is managed by ${owner.folder}`,
          dependents: [owner.folder],
        },
      };
    }

    // Check for reverse dependencies (addons that require this one)
    if (!this.force) {
      const dependents = this.dbManager.getRequiredDependents(this.folder);
      if (dependents.length > 0) {
        return {
          success: false,
          removedFolders: [],
          blocked: {
            reason: "This addon is required by other addons",
            dependents: dependents.map((d) => d.folder),
          },
        };
      }
    }

    const addonsDir = this.configManager.get().destDir;

    // Collect all folders to remove (main folder + owned folders)
    const foldersToRemove = [
      this.folder,
      ...(this.addonRecord.ownedFolders || []),
    ];

    // 1. Create backups for all folders
    const tempDir = path.join(os.tmpdir(), "lemonup-backups");
    await fs.mkdir(tempDir, { recursive: true });

    for (const folder of foldersToRemove) {
      const addonPath = path.join(addonsDir, folder);
      const backupPath = path.join(tempDir, `${folder}-${Date.now()}`);

      try {
        await fs.cp(addonPath, backupPath, { recursive: true });
        this.backupPaths.set(folder, backupPath);
      } catch (error) {
        logger.error("Manager", `Failed to create backup for ${folder}`, error);
        // Continue with removal even if backup fails
      }
    }

    // 2. Remove main addon from DB (owned folders don't have DB records)
    this.dbManager.removeAddon(this.folder);

    // 3. Remove from Config (sync)
    this.configManager.removeRepository(this.addonRecord.name);

    // 4. Remove all folders from Disk
    const removedFolders: string[] = [];
    try {
      for (const folder of foldersToRemove) {
        const addonPath = path.join(addonsDir, folder);
        await fs.rm(addonPath, { recursive: true, force: true });
        removedFolders.push(folder);
      }
      context.emit("addon:remove:complete", this.folder);
      return { success: true, removedFolders };
    } catch (error) {
      logger.error(
        "Manager",
        `Failed to delete addon folders for: ${this.folder}`,
        error,
      );
      // Try to rollback immediately if disk removal fails
      await this.undo(context);
      return { success: false, removedFolders: [] };
    }
  }

  async undo(_context: CommandContext): Promise<void> {
    if (!this.addonRecord) return;

    logger.log("Manager", `Rolling back removal of ${this.folder}`);

    const addonsDir = this.configManager.get().destDir;

    try {
      // Restore all folders from backups
      for (const [folder, backupPath] of this.backupPaths) {
        const addonPath = path.join(addonsDir, folder);
        await fs.cp(backupPath, addonPath, { recursive: true });
      }

      // Restore DB record
      this.dbManager.addAddon(this.addonRecord);
    } catch (error) {
      logger.error("Manager", `Rollback failed for ${this.folder}`, error);
    }
  }
}
