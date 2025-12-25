import fs from "node:fs/promises";
import path from "node:path";
import type { ConfigManager } from "@/core/config";
import type { AddonRecord, DatabaseManager } from "@/core/db";
import * as GitClient from "@/core/git";
import { logger } from "@/core/logger";
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

		const tocGlob = new Bun.Glob("*/*.toc");
		let count = 0;
		const folderDeps = new Map<string, string[]>();

		for await (const file of tocGlob.scan({ cwd: addonsDir })) {
			const folderName = path.dirname(file);
			if (!folderName || folderName === ".") continue;

			if (this.specificFolders && !this.specificFolders.includes(folderName)) {
				continue;
			}

			context.emit("scan:progress", folderName);

			const fullPath = path.join(addonsDir, file);

			try {
				const content = await Bun.file(fullPath).text();

				const titleMatch = content.match(/^## Title:\s*(.*)/m);
				const versionMatch = content.match(/^## Version:\s*(.*)/m);
				const authorMatch = content.match(/^## Author:\s*(.*)/m);
				const interfaceMatch = content.match(/^## Interface:\s*(.*)/m);
				const depsMatch = content.match(
					/^## (?:Dependencies|RequiredDeps):\s*(.*)/m,
				);

				const title = titleMatch?.[1]?.trim() ?? folderName;
				const cleanTitle = title.replace(/\|c[0-9a-fA-F]{8}(.*?)\|r/g, "$1");

				const version = versionMatch?.[1]?.trim() ?? null;
				const author = authorMatch?.[1]?.trim() ?? null;
				const gameInterface = interfaceMatch?.[1]?.trim() ?? null;

				const deps = depsMatch?.[1]?.split(/,\s*|\s+/).filter(Boolean) || [];
				if (deps.length > 0) {
					folderDeps.set(folderName, deps);
				}

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
						ownedFolders: [],
						kind: "addon",
						kindOverride: false,
						flavor: "retail",
						requiredDeps: [],
						optionalDeps: [],
						embeddedLibs: [],
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

		// TODO (Task 5): Replace old parent-child logic with new ownedFolders + dependencies model
		// Old parent-child relationship logic removed - will be replaced in Task 5

		context.emit("scan:complete", count);
		return count;
	}
}
