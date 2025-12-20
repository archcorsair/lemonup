import fs from "node:fs/promises";
import path from "node:path";
import type { ConfigManager } from "../config";
import type { AddonRecord, DatabaseManager } from "../db";
import * as GitClient from "../git";
import { logger } from "../logger";
import type { Command, CommandContext } from "./types";

export class ScanCommand implements Command<number> {
	constructor(
		private dbManager: DatabaseManager,
		private configManager: ConfigManager,
		private specificFolders?: string[],
	) {}

	async execute(context: CommandContext): Promise<number> {
		context.emit("scan:start");
		const addonsDir = this.configManager.get().destDir;
		let exists = false;
		try {
			await fs.access(addonsDir);
			exists = true;
		} catch {
			exists = false;
		}

		if (!exists) {
			logger.log("Manager", `Addons directory not found: ${addonsDir}`);
			context.emit("scan:complete", 0);
			return 0;
		}

		if (!this.specificFolders) {
			const IGNORED_FOLDERS = [
				"ElvUI_OptionsUI",
				"ElvUI_Options",
				"ElvUI_Libraries",
			];
			for (const ignored of IGNORED_FOLDERS) {
				if (this.dbManager.getByFolder(ignored)) {
					this.dbManager.removeAddon(ignored);
				}
			}
		}

		const tocGlob = new Bun.Glob("*/*.toc");
		let count = 0;

		for await (const file of tocGlob.scan({ cwd: addonsDir })) {
			const folderName = path.dirname(file);
			if (!folderName || folderName === ".") continue;

			if (this.specificFolders && !this.specificFolders.includes(folderName)) {
				continue;
			}

			if (
				folderName === "ElvUI_OptionsUI" ||
				folderName === "ElvUI_Options" ||
				folderName === "ElvUI_Libraries"
			)
				continue;

			context.emit("scan:progress", folderName);

			const fullPath = path.join(addonsDir, file);

			try {
				const content = await Bun.file(fullPath).text();

				const titleMatch = content.match(/^## Title:\s*(.*)/m);
				const versionMatch = content.match(/^## Version:\s*(.*)/m);
				const authorMatch = content.match(/^## Author:\s*(.*)/m);
				const interfaceMatch = content.match(/^## Interface:\s*(.*)/m);

				const title = titleMatch?.[1]?.trim() ?? folderName;
				const cleanTitle = title.replace(/\|c[0-9a-fA-F]{8}(.*?)\|r/g, "$1");

				const version = versionMatch?.[1]?.trim() ?? null;
				const author = authorMatch?.[1]?.trim() ?? null;
				const gameInterface = interfaceMatch?.[1]?.trim() ?? null;

				const gitPath = path.join(addonsDir, folderName, ".git");
				let isGit = false;
				let gitHash: string | null = null;
				try {
					await fs.stat(gitPath);
					isGit = true;
					gitHash = await GitClient.getCurrentCommit(
						path.join(addonsDir, folderName),
					);
				} catch {
					isGit = false;
				}

				const finalVersion =
					version || (isGit && gitHash ? gitHash.substring(0, 7) : "Unknown");

				const existing = this.dbManager.getByFolder(folderName);

				if (existing) {
					const updates: Partial<AddonRecord> = {};
					let updated = false;

					if (isGit && existing.type !== "github") {
						updates.type = "github";
						updated = true;
					}

					if (finalVersion && existing.version !== finalVersion) {
						updates.version = finalVersion;
						updated = true;
					}

					if (gitHash && existing.git_commit !== gitHash) {
						updates.git_commit = gitHash;
						updated = true;
					}

					if (author && existing.author !== author) {
						updates.author = author;
						updated = true;
					}
					if (gameInterface && existing.interface !== gameInterface) {
						updates.interface = gameInterface;
						updated = true;
					}

					if (updated) {
						this.dbManager.updateAddon(folderName, updates);
					}
				} else {
					this.dbManager.addAddon({
						name: cleanTitle,
						folder: folderName,
						version: finalVersion,
						git_commit: gitHash,
						author: author,
						interface: gameInterface,
						url: null,
						type: "manual",
						parent: null,
						install_date: new Date().toISOString(),
						last_updated: new Date().toISOString(),
					});
					count++;
				}
			} catch (e) {
				context.emit(
					"error",
					`Scan:${folderName}`,
					e instanceof Error ? e.message : String(e),
				);
			}
		}

		context.emit("scan:complete", count);
		return count;
	}
}
