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

			const scanCmd = new ScanCommand(
				this.dbManager,
				this.configManager,
				installedNames,
			);
			await scanCmd.execute(context);

			let mainAddonName: string | null = null;
			let targetName = "";

			if (repoType === "github") {
				const pathname = parsed.pathname.replace(/\.git$/, "");
				targetName = pathname.split("/").pop() || "";
			} else if (repoType === "wowinterface" && wowInterfaceDetails !== null) {
				// Use filename without extension as primary guess, or UIName
				targetName = wowInterfaceDetails.UIFileName.replace(/\.zip$/i, "");
			}

			// Find Parent
			// Exact match (case-insensitive)
			mainAddonName =
				installedNames.find(
					(name) => name.toLowerCase() === targetName.toLowerCase(),
				) || null;

			// If WoWInterface, try matching UIName
			if (
				!mainAddonName &&
				repoType === "wowinterface" &&
				wowInterfaceDetails
			) {
				mainAddonName =
					installedNames.find(
						(name) =>
							name.toLowerCase() === wowInterfaceDetails.UIName.toLowerCase(),
					) || null;
			}

			// Fallback: If one folder is a prefix of others? (ElvUI vs ElvUI_Config)
			if (!mainAddonName && installedNames.length > 1) {
				// Sort by length, shortest first.
				const sorted = [...installedNames].sort((a, b) => a.length - b.length);

				// Heuristic 1: Shortest folder is prefix of MOST others (allow some deviations like Libs)
				const shortest = sorted[0];
				if (shortest) {
					const prefixCount = sorted.filter((n) =>
						n.startsWith(shortest),
					).length;
					// If shortest is prefix for at least 50% of items, assume it's parent
					if (prefixCount / sorted.length >= 0.5) {
						mainAddonName = shortest;
					}
				}

				// Heuristic 2: Target Name Similarity
				// If we still don't have one, check if any folder is a substring of targetName
				// e.g. target="Details-Damage-Meter", folder="Details"
				if (!mainAddonName && targetName) {
					const candidates = installedNames.filter(
						(name) =>
							targetName.toLowerCase().includes(name.toLowerCase()) ||
							name.toLowerCase().includes(targetName.toLowerCase()),
					);
					// Pick the shortest candidate that matches (likely the root name)
					if (candidates.length > 0) {
						candidates.sort((a, b) => a.length - b.length);
						mainAddonName = candidates[0] || null;
					}
				}
			}

			if (repoType === "github") {
				const installedHash =
					(await GitClient.getCurrentCommit(tempDir)) || null;
				for (const addonName of installedNames) {
					// TODO: Handle multi-folder addons via ownedFolders
					this.dbManager.updateAddon(addonName, {
						url: this.url,
						type: REPO_TYPE.GITHUB,
						git_commit: installedHash,
						last_updated: new Date().toISOString(),
					});
				}
			} else if (repoType === "wowinterface" && wowInterfaceDetails !== null) {
				for (const addonName of installedNames) {
					// TODO: Handle multi-folder addons via ownedFolders
					this.dbManager.updateAddon(addonName, {
						url: this.url,
						type: REPO_TYPE.WOWINTERFACE,
						version: wowInterfaceDetails.UIVersion,
						author: wowInterfaceDetails.UIAuthorName,
						last_updated: new Date().toISOString(),
					});
				}
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
}
