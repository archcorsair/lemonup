import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { ConfigManager } from "../config";
import type { AddonRecord, DatabaseManager } from "../db";
import * as Downloader from "../downloader";
import * as GitClient from "../git";
import { logger } from "../logger";
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
	                context.emit("addon:update-check:start", folder);
	
	                this.previousRecord = { ...this.addon };
	
	                let remoteVersion = "unknown";
	                let updateAvailable = true;
	
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
	                                updateAvailable = localHash !== remoteHash;
	                        } catch (err) {
	                                return {
	                                        repoName: name,
	                                        success: false,
	                                        updated: false,
	                                        error: String(err),
	                                };
	                        }
	                } else if (this.addon.type === "tukui") {
	                        if (name === "ElvUI" || folder === "ElvUI") {
	                                const hashMatch = this.addon.version?.match(/-g([a-f0-9]+)/);
	                                const localHash =
	                                        this.addon.git_commit || (hashMatch ? hashMatch[1] : null);
	
	                                const remoteHash = await GitClient.getRemoteCommit(
	                                        "https://github.com/tukui-org/ElvUI",
	                                        "main",
	                                );
	
	                                if (remoteHash) {
	                                        remoteVersion = remoteHash;
	                                        if (
	                                                localHash &&
	                                                (remoteHash.startsWith(localHash) ||
	                                                        localHash.startsWith(remoteHash))
	                                        ) {
	                                                updateAvailable = false;
	                                        } else {
	                                                updateAvailable = true;
	                                        }
	                                } else {
	                                        updateAvailable = true;
	                                        remoteVersion = "latest";
	                                }
	                        } else {
	                                updateAvailable = true;
	                                remoteVersion = "latest";
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
	                        if (this.addon.type === "tukui") {
	                                context.emit("addon:install:downloading", folder);
	                                const zipPath = path.join(tempDir, "addon.zip");
	                                if (!(await Downloader.download(this.addon.url || "", zipPath))) {
	                                        throw new Error("Download failed");
	                                }
	
	                                context.emit("addon:install:extracting", folder);
	                                const extractPath = path.join(tempDir, "extract");
	                                await fs.mkdir(extractPath, { recursive: true });
	                                if (!(await Downloader.unzip(zipPath, extractPath))) {
	                                        throw new Error("Unzip failed");
	                                }
	
	                                await this.backupAndInstall(
	                                        context,
	                                        path.join(extractPath, folder),
	                                        folder,
	                                );
	                        } else if (this.addon.type === "github") {
	                                context.emit("addon:install:downloading", folder);
	                                if (!(await GitClient.clone(this.addon.url || "", "main", tempDir))) {
	                                        throw new Error("Git Clone failed");
	                                }
	
	                                await this.backupAndInstall(
	                                        context,
	                                        path.join(tempDir, folder),
	                                        folder,
	                                );
	                        }
	
	                        const isGitHash = remoteVersion.match(/^[a-f0-9]{40}$/);
	
	                        this.dbManager.updateAddon(folder, {
	                                version: isGitHash ? remoteVersion.substring(0, 7) : remoteVersion,
	                                git_commit: isGitHash ? remoteVersion : null,
	                                last_updated: new Date().toISOString(),
	                        });
	
	                        context.emit("addon:install:complete", folder);
	                        return {
	                                repoName: name,
	                                success: true,
	                                updated: true,
	                                message: `Updated to ${remoteVersion.substring(0, 7)}`,
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
	                } catch {}
	
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
				last_updated: this.previousRecord.last_updated,
			});
		}
	}
}
