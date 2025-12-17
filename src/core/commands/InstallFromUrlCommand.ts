import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "../config";
import type { DatabaseManager } from "../db";
import * as GitClient from "../git";
import { logger } from "../logger";
import { ScanCommand } from "./ScanCommand";
import type { Command, CommandContext } from "./types";

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
		const tempDir = path.join(os.tmpdir(), "lemonup-install-" + Date.now());

		try {
			// Validate URL
			const parsed = new URL(this.url);
			if (!parsed.hostname.endsWith("github.com")) {
				throw new Error("Only github.com URLs are supported");
			}

			logger.log("InstallFromUrlCommand", `Installing from URL: ${this.url}`);
			context.emit("addon:install:start", this.url);

			// Clone
			context.emit("addon:install:downloading", this.url);
			if (!(await GitClient.clone(this.url, "main", tempDir))) {
				throw new Error("Git Clone failed");
			}

			// Scan for folders containing .toc
			const tocGlob = new Bun.Glob("**/*.toc");
			const foundFolders = new Set<string>();

			for await (const file of tocGlob.scan({ cwd: tempDir })) {
				const dir = path.dirname(file);
				if (dir !== ".") {
					foundFolders.add(dir);
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
				for (const folder of foldersToCopy) {
					const folderName = path.basename(folder);
					const source = path.join(tempDir, folder);
					const dest = path.join(config.destDir, folderName);
					await fs.cp(source, dest, { recursive: true, force: true });
					installedNames.push(folderName);
					this.installedFolders.push(folderName);
				}
			}

			if (installedNames.length === 0) {
				throw new Error("No addons found in repository");
			}

			// Register in DB using ScanCommand
			const scanCmd = new ScanCommand(this.dbManager, this.configManager, installedNames);
			await scanCmd.execute(context);

			// Update records with Git Metadata
			for (const addonName of installedNames) {
				this.dbManager.updateAddon(addonName, {
					url: this.url,
					type: "github",
					last_updated: new Date().toISOString(),
				});
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

	async undo(context: CommandContext): Promise<void> {
		logger.log("InstallFromUrlCommand", "Rolling back installation");
		const destDir = this.configManager.get().destDir;
		for (const folder of this.installedFolders) {
			try {
				await fs.rm(path.join(destDir, folder), { recursive: true, force: true });
				this.dbManager.removeAddon(folder);
			} catch (err) {
				logger.error("InstallFromUrlCommand", `Failed to remove ${folder} during undo`, err);
			}
		}
	}
}
