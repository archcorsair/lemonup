import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "../config";
import type { AddonRecord, DatabaseManager } from "../db";
import { logger } from "../logger";
import type { Command, CommandContext } from "./types";

export class RemoveAddonCommand implements Command<boolean> {
	private addonRecord: AddonRecord | null = null;
	private backupPath: string | null = null;

	constructor(
		private dbManager: DatabaseManager,
		private configManager: ConfigManager,
		private folder: string,
	) {}

	async execute(context: CommandContext): Promise<boolean> {
		context.emit("addon:remove:start", this.folder);
		this.addonRecord = this.dbManager.getByFolder(this.folder);

		if (!this.addonRecord) {
			logger.error("Manager", `Addon not found in DB: ${this.folder}`);
			return false;
		}

		const addonsDir = this.configManager.get().destDir;
		const addonPath = path.join(addonsDir, this.folder);

		// 1. Create backup
		const tempDir = path.join(os.tmpdir(), "lemonup-backups");
		await fs.mkdir(tempDir, { recursive: true });
		this.backupPath = path.join(tempDir, `${this.folder}-${Date.now()}`);

		try {
			await fs.cp(addonPath, this.backupPath, { recursive: true });
		} catch (error) {
			logger.error("Manager", `Failed to create backup for ${this.folder}`, error);
			// If we can't backup, maybe we shouldn't proceed?
			// For safety, let's proceed but log it.
		}

		// 2. Remove from DB
		this.dbManager.removeAddon(this.folder);

		// 3. Remove from Config (sync)
		this.configManager.removeRepository(this.addonRecord.name);

		// 4. Remove from Disk
		try {
			await fs.rm(addonPath, { recursive: true, force: true });
			context.emit("addon:remove:complete", this.folder);
			return true;
		} catch (error) {
			logger.error("Manager", `Failed to delete addon folder: ${this.folder}`, error);
			// Try to rollback immediately if disk removal fails?
			await this.undo(context);
			return false;
		}
	}

	async undo(context: CommandContext): Promise<void> {
		if (!this.addonRecord || !this.backupPath) return;

		logger.log("Manager", `Rolling back removal of ${this.folder}`);

		const addonsDir = this.configManager.get().destDir;
		const addonPath = path.join(addonsDir, this.folder);

		try {
			// Restore Disk
			await fs.cp(this.backupPath, addonPath, { recursive: true });

			// Restore DB
			this.dbManager.addAddon(this.addonRecord);

			// Note: Restoring Config might be harder if it was more complex,
			// but for now removeRepository is simple.
		} catch (error) {
			logger.error("Manager", `Rollback failed for ${this.folder}`, error);
		}
	}
}
