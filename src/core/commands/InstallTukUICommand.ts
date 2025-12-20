import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "@/core/config";
import type { DatabaseManager } from "@/core/db";
import * as Downloader from "@/core/downloader";
import { logger } from "@/core/logger";
import { isPathConfigured } from "@/core/paths";
import * as TukUI from "@/core/tukui";
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

		const tempDir = path.join(os.tmpdir(), `lemonup-tukui-${Date.now()}`);
		await fs.mkdir(tempDir, { recursive: true });

		context.emit("addon:install:start", this.addonFolder);

		try {
			let details: TukUI.TukUIAddon | null = null;
			try {
				details = await TukUI.getAddonDetails(this.addonFolder);
			} catch (e) {
				logger.error("InstallTukUICommand", "Failed to fetch TukUI details", e);
			}

			let downloadUrl = this.url;
			if ((!downloadUrl || downloadUrl === "latest") && details) {
				downloadUrl = details.url;
			}

			if (!downloadUrl) {
				throw new Error("No download URL provided or found");
			}

			context.emit("addon:install:downloading", this.addonFolder);
			const zipPath = path.join(tempDir, "addon.zip");
			if (!(await Downloader.download(downloadUrl, zipPath))) {
				throw new Error("Download failed");
			}

			context.emit("addon:install:extracting", this.addonFolder);
			const extractPath = path.join(tempDir, "extract");
			await fs.mkdir(extractPath, { recursive: true });
			if (!(await Downloader.unzip(zipPath, extractPath))) {
				throw new Error("Unzip failed");
			}

			context.emit("addon:install:copying", this.addonFolder);
			const foldersToCopy = [this.addonFolder, ...this.subFolders];
			for (const folder of foldersToCopy) {
				const source = path.join(extractPath, folder);
				const dest = path.join(config.destDir, folder);
				await fs.cp(source, dest, { recursive: true, force: true });
			}

			const scanCmd = new ScanCommand(
				this.dbManager,
				this.configManager,
				foldersToCopy,
			);
			await scanCmd.execute(context);

			// Update Main Addon
			this.dbManager.updateAddon(this.addonFolder, {
				type: "tukui",
				url: downloadUrl,
				version: details?.version || "unknown",
				author: details?.author || null,
				last_updated: new Date().toISOString(),
				parent: null,
				git_commit: null,
			});

			for (const subFolder of this.subFolders) {
				this.dbManager.updateAddon(subFolder, {
					type: "tukui",
					url: downloadUrl,
					version: details?.version || "unknown",
					author: details?.author || null,
					last_updated: new Date().toISOString(),
					parent: this.addonFolder,
					git_commit: null,
				});
			}

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
