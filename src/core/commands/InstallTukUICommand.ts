import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "../config";
import type { DatabaseManager } from "../db";
import * as Downloader from "../downloader";
import { isPathConfigured } from "../paths";
import { ScanCommand } from "./ScanCommand";
import type { Command, CommandContext } from "./types";

export class InstallTukUICommand implements Command<boolean> {
	constructor(
		private dbManager: DatabaseManager,
		private configManager: ConfigManager,
		private url: string,
		private addonFolder: string,
		private subFolders: string[] = [],
	) {}

	async execute(context: CommandContext): Promise<boolean> {
		const config = this.configManager.get();

		if (!isPathConfigured(config.destDir)) {
			throw new Error("WoW Addon directory is not configured.");
		}

		const tempDir = path.join(os.tmpdir(), "lemonup-tukui-" + Date.now());
		await fs.mkdir(tempDir, { recursive: true });

		context.emit("addon:install:start", this.addonFolder);

		try {
			// Download
			context.emit("addon:install:downloading", this.addonFolder);
			const zipPath = path.join(tempDir, "addon.zip");
			if (!(await Downloader.download(this.url, zipPath))) {
				throw new Error("Download failed");
			}

			// Extract
			context.emit("addon:install:extracting", this.addonFolder);
			const extractPath = path.join(tempDir, "extract");
			await fs.mkdir(extractPath, { recursive: true });
			if (!(await Downloader.unzip(zipPath, extractPath))) {
				throw new Error("Unzip failed");
			}

			// Copy
			context.emit("addon:install:copying", this.addonFolder);
			const foldersToCopy = [this.addonFolder, ...this.subFolders];
			for (const folder of foldersToCopy) {
				const source = path.join(extractPath, folder);
				const dest = path.join(config.destDir, folder);
				await fs.cp(source, dest, { recursive: true, force: true });
			}

			// Register
			const scanCmd = new ScanCommand(this.dbManager, this.configManager, [this.addonFolder]);
			await scanCmd.execute(context);

			// Update type/url
			this.dbManager.updateAddon(this.addonFolder, {
				type: "tukui",
				url: this.url,
				last_updated: new Date().toISOString(),
			});

			context.emit("addon:install:complete", this.addonFolder);
			return true;
		} catch (error) {
			logger.error("InstallTukUICommand", "Failed", error);
			throw error;
		} finally {
			await fs.rm(tempDir, { recursive: true, force: true });
		}
	}
}

import { logger } from "../logger";
