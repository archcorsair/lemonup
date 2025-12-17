import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
	type Config,
	ConfigManager,
	REPO_TYPE,
	type Repository,
} from "./config";
import { type AddonRecord, DatabaseManager } from "./db";
import * as Downloader from "./downloader";
import * as GitClient from "./git";
import { logger } from "./logger";

export interface UpdateResult {
	repoName: string;
	success: boolean;
	updated: boolean;
	message?: string;
	error?: string;
}

export class AddonManager {
	private configManager: ConfigManager;
	private dbManager: DatabaseManager;

	constructor(configManager?: ConfigManager) {
		this.configManager = configManager || new ConfigManager();
		// Use the same directory as the config file for the database
		const configDir = path.dirname(this.configManager.path);
		this.dbManager = new DatabaseManager(configDir);

		this.migrateConfig();
	}

	public getConfig(): Config {
		return this.configManager.get();
	}

	public close() {
		this.dbManager.close();
	}

	private migrateConfig() {
		const config = this.configManager.get();
		if (config.repositories && config.repositories.length > 0) {
			logger.log(
				"Manager",
				"Migrating repositories from config to database...",
			);
			for (const repo of config.repositories) {
				const folder = repo.folders[0]; // Assumption: use first folder as ID/primary key equivalent for now
				if (!folder) continue;

				// Check if exists
				if (this.dbManager.getByFolder(folder)) continue;

				this.dbManager.addAddon({
					name: repo.name,
					folder: folder,
					version: repo.installedVersion,
					author: null, // Config didn't store author
					interface: null, // Config didn't store interface
					url: repo.gitRemote || repo.downloadUrl || null,
					type: repo.type as "github" | "tukui",
					install_date: new Date().toISOString(),
					last_updated: new Date().toISOString(),
				});
				logger.log("Manager", `Migrated ${repo.name}`);
			}

			// Optional: clear config repositories to prevent re-migration?
			// For safety, let's keep them one time, or user can clean up.
			// Ideally we modify schema to remove 'repositories' but zod validation would fail.
			// Let's just leave them for now but ignore them.
		}
	}

	/**
	 * Main entry point to update all repositories.
	 */
	public async updateAll(force = false): Promise<UpdateResult[]> {
		const config = this.configManager.get();
		const addons = this.dbManager.getAll();
		const results: UpdateResult[] = [];

		// Ensure temp dir exists
		const tempDir = path.join(os.tmpdir(), "lemonup");
		await fs.mkdir(tempDir, { recursive: true });

		for (const addon of addons) {
			// Skip manual addons for update check implies (?)
			// For now only update github/tukui
			if (addon.type === "manual") continue;

			try {
				const result = await this.updateAddon(addon, config, tempDir, force);
				results.push(result);
			} catch (error) {
				results.push({
					repoName: addon.name,
					success: false,
					updated: false,
					error: String(error),
				});
			}
		}

		// Cleanup temp dir
		await fs.rm(tempDir, { recursive: true, force: true });

		return results;
	}

	public async checkUpdate(addon: AddonRecord): Promise<{
		updateAvailable: boolean;
		remoteVersion: string;
		error?: string;
	}> {
		// Only check if we have a URL
		if (!addon.url) {
			return {
				updateAvailable: false,
				remoteVersion: "",
				error: "Missing URL",
			};
		}

		if (addon.type === "github") {
			// For GitHub, url is gitRemote. branch? Config used 'branch'.
			// Migration didn't capture branch. We need branch in DB!
			// FIXME: Add branch to schema or assume main?
			// Let's assume main for now or we need to add branch column.
			// User plan was just the ID/Name/etc.
			// Let's use "main" as default as per Config.
			const branch = "main";
			const remoteHash = await GitClient.getRemoteCommit(addon.url, branch);
			if (!remoteHash) {
				return {
					updateAvailable: false,
					remoteVersion: "",
					error: "Failed to get remote hash",
				};
			}

			return {
				updateAvailable: addon.version !== remoteHash,
				remoteVersion: remoteHash,
			};
		}

		// For TukUI
		if (addon.type === "tukui") {
			// Special handling for ElvUI to check GitHub commits (Dev Version)
			if (addon.name === "ElvUI" || addon.folder === "ElvUI") {
				// Try to extract git hash from TOC version (e.g. v13.08-1-g123abc)
				const hashMatch = addon.version?.match(/-g([a-f0-9]+)/);
				const localHash = hashMatch ? hashMatch[1] : null;

				const remoteHash = await GitClient.getRemoteCommit(
					"https://github.com/tukui-org/ElvUI",
					"main",
				);

				logger.error(
					"Manager",
					`[ElvUI Debug] Version: '${addon.version}', LocalHash: '${localHash}', RemoteHash: '${remoteHash}'`,
				);

				if (remoteHash && localHash) {
					// Check if remote full hash starts with local short hash
					if (remoteHash.startsWith(localHash)) {
						logger.error("Manager", `[ElvUI Debug] Hashes match.`);
						return {
							updateAvailable: false,
							remoteVersion: remoteHash,
						};
					}
					logger.error("Manager", `[ElvUI Debug] Hash mismatch.`);
					return {
						updateAvailable: true,
						remoteVersion: remoteHash,
					};
				}

				// If we couldn't parse local hash but have remote, assume update?
				// Or if parsing failed, maybe it's a Release version?
				// Fallback to Tag check if hash check fails/not applicable?
				// User specifically asked for hash check for Dev.
				// Let's fallback to simplified "Latest" if we can't compare.
				if (remoteHash) {
					return {
						updateAvailable: true, // conservative
						remoteVersion: remoteHash,
					};
				}
			}

			// Fallback for other TukUI addons or if check fails
			return { updateAvailable: true, remoteVersion: "latest" };
		}

		return { updateAvailable: false, remoteVersion: "" };
	}

	public async updateAddon(
		addon: AddonRecord,
		config: Config,
		tempBaseDir: string,
		force: boolean,
		dryRun = false,
		onProgress?: (status: string) => void,
	): Promise<UpdateResult> {
		const { name } = addon;
		logger.log(
			"Manager",
			`Starting updateAddon for ${name}. Force=${force}, DryRun=${dryRun}`,
		);

		onProgress?.("checking");

		let remoteVersion = "unknown";

		// Only check if Github for now or if we have a git-like URL
		if (addon.type === "github") {
			const check = await this.checkUpdate(addon);
			if (check.error) {
				return {
					repoName: name,
					success: false,
					updated: false,
					error: check.error,
				};
			}
			if (!force && !check.updateAvailable) {
				return {
					repoName: name,
					success: true,
					updated: false,
					message: "Up to date",
				};
			}
			remoteVersion = check.remoteVersion;
		}

		// Perform Update
		onProgress?.("downloading");

		if (dryRun) {
			await new Promise((resolve) => setTimeout(resolve, 2000));
			return {
				repoName: name,
				success: true,
				updated: true,
				message: `[Dry Run] Would update to ${remoteVersion.substring(0, 7)}`,
			};
		}

		const repoTempDir = path.join(tempBaseDir, name);
		await fs.rm(repoTempDir, { recursive: true, force: true });

		try {
			if (addon.type === "tukui") {
				if (!addon.url) throw new Error("Missing URL for TukUI addon");
				await this.installTukUI(
					addon.url || "https://api.tukui.org/v1/download/dev/elvui/main", // Fallback if no URL
					config,
					repoTempDir, // Use repoTempDir here
					[addon.folder],
				);
			} else if (addon.type === "github") {
				if (!addon.url) throw new Error("Missing git URL");
				await this.installGithub(addon.url, "main", config, repoTempDir, [
					addon.folder,
				]);
			} else {
				throw new Error(`Unknown type: ${addon.type}`);
			}

			// Update DB
			logger.log("Manager", `Updating DB for ${name} to ${remoteVersion}`);
			this.dbManager.updateAddon(addon.folder, {
				version: remoteVersion,
				last_updated: new Date().toISOString(),
			});

			return {
				repoName: name,
				success: true,
				updated: true,
				message: `Updated to ${remoteVersion.substring(0, 7)}`,
			};
		} catch (err: unknown) {
			logger.error("Manager", `Error updating ${name}`, err);
			return {
				repoName: name,
				success: false,
				updated: false,
				error: err instanceof Error ? err.message : String(err),
			};
		}
	}

	public async installTukUI(
		url: string,
		config: Config,
		tempDir: string,
		folders: string[],
	): Promise<boolean> {
		await fs.mkdir(tempDir, { recursive: true });
		const zipPath = path.join(tempDir, "addon.zip");

		try {
			// Download
			if (!(await Downloader.download(url, zipPath))) {
				throw new Error("Download failed");
			}

			// Extract
			const extractPath = path.join(tempDir, "extract");
			await fs.mkdir(extractPath, { recursive: true });
			if (!(await Downloader.unzip(zipPath, extractPath))) {
				throw new Error("Unzip failed");
			}

			// Install (Copy)
			return await this.copyFolders(extractPath, config.destDir, folders);
		} finally {
			// Cleanup temp dir
			await fs.rm(tempDir, { recursive: true, force: true });
		}
	}

	private async installGithub(
		url: string,
		branch: string,
		config: Config,
		tempDir: string,
		folders: string[],
	): Promise<boolean> {
		// Clone directly to tempDir
		if (!(await GitClient.clone(url, branch, tempDir))) {
			throw new Error("Git Clone failed");
		}

		// Install (Copy)
		return await this.copyFolders(tempDir, config.destDir, folders);
	}

	private async copyFolders(
		sourceBase: string,
		destBase: string,
		folders: string[],
	): Promise<boolean> {
		try {
			// Ensure destination base directory exists (important for Test Mode or fresh installs)
			await fs.mkdir(destBase, { recursive: true });

			for (const folder of folders) {
				const source = path.join(sourceBase, folder);
				const dest = path.join(destBase, folder);

				// Ensure source exists
				const sourceStat = await fs.stat(source).catch(() => null);
				if (!sourceStat || !sourceStat.isDirectory()) {
					console.error(
						`Source folder not found or not a directory: ${source}`,
					);
					return false;
				}

				// Copy recursively using native fs.cp for cross-platform support
				await fs.cp(source, dest, { recursive: true, force: true });
			}
			return true;
		} catch (error) {
			// console.error("Copy failed:", error);
			// Don't log, throw to propagate to UI
			throw new Error(
				`Copy failed: ${error instanceof Error ? error.message : String(error)}`,
			);
		}
	}

	public async installFromUrl(
		url: string,
	): Promise<{ success: boolean; installedAddons: string[]; error?: string }> {
		const config = this.configManager.get();
		const tempDir = path.join(os.tmpdir(), "lemonup-install-" + Date.now());

		try {
			// Validate URL
			try {
				const parsed = new URL(url);
				if (!parsed.hostname.endsWith("github.com")) {
					throw new Error("Only github.com URLs are supported");
				}
			} catch (e) {
				if (e instanceof Error && e.message.includes("Only github.com"))
					throw e;
				throw new Error(
					"Invalid URL: Must be a valid HTTPS URL (e.g. https://github.com/user/repo)",
				);
			}

			logger.log("Manager", `Installing from URL: ${url}`);

			// Clone
			if (!(await GitClient.clone(url, "main", tempDir))) {
				throw new Error("Git Clone failed");
			}

			// Scan for folders to install
			// We look for any folder containing a .toc file
			const tocGlob = new Bun.Glob("**/*.toc");
			const foundFolders = new Set<string>();

			for await (const file of tocGlob.scan({ cwd: tempDir })) {
				// file is relative, e.g. "MyAddon/MyAddon.toc"
				// We want the top-level folder usually?
				// Actually, addons are usually at root or in a subfolder.
				// If we clone "Repo", and it contains "AddonA/AddonA.toc", we want to copy "AddonA".
				// If it contains "AddonB.toc" at root... wait.
				// WoW Addons: Each folder in Interface/AddOns must have a matching .toc inside it.
				// So if we have "Repo/AddonA/AddonA.toc", we copy "AddonA" to "Interface/AddOns/AddonA".

				const dir = path.dirname(file);
				if (dir === ".") {
					// TOC at root. This means the REPO itself is the addon.
					// But we cloned to a temp dir with random name.
					// We need to know the intended name.
					// Usually repo name? Or TOC title?
					// Let's parse TOC title/folder usage.
					// For now, if TOC at root, we might need to copy the *contents* of tempDir to "Dest/RepoName"?
					// But URL might not give repo name easily if it's raw.
					// Let's assume standard structure: folders inside repo.
					// If TOC at root, we use the TOC name or just specific logic.
					// Let's just gather all dirs that HAVE a toc.
					// If file is "MyAddon.toc", dir is ".".
				} else {
					// "SubFolder/MyAddon.toc" -> dir is "SubFolder".
					// We take the top-most folder relative to tempDir?
					// Usually repositories are:
					// Root/
					//   Addon1/
					//     Addon1.toc
					//   Addon2/
					//     Addon2.toc
					// In this case we copy Addon1 and Addon2.

					// What about nested? Root/Collection/Addon1...
					// Recursion is tricky.
					// Simplified rule: Copy any directory that directly contains a .toc file.

					foundFolders.add(dir);
				}
			}

			const foldersToCopy = Array.from(foundFolders);
			// Handle root case: if foldersToCopy is empty, maybe root has toc?
			// If root has toc, we can't easily "copy root folder" because root is tempDir.
			// We would need to create a folder in Dest based on TOC Title or Repo Name.
			// Let's implement Root TOC support later if needed. Most "Collections" repos separate them.
			// Single addon repos often have .toc at root.

			// Check for root .toc
			const rootTocs = Array.from(
				new Bun.Glob("*.toc").scanSync({ cwd: tempDir }),
			);
			const installedNames: string[] = [];

			if (rootTocs.length > 0) {
				// Root has .toc. The Addon Name is likely the .toc filename (minus extension).
				// e.g. "DeadlyBossMods.toc" -> Folder should be "DeadlyBossMods".
				for (const tocFile of rootTocs) {
					const addonName = path.basename(tocFile, ".toc");
					const dest = path.join(config.destDir, addonName);

					logger.log("Manager", `Installing root addon: ${addonName}`);
					await fs.cp(tempDir, dest, { recursive: true, force: true });
					installedNames.push(addonName);
				}
			}

			if (foldersToCopy.length > 0) {
				await this.copyFolders(tempDir, config.destDir, foldersToCopy);
				installedNames.push(...foldersToCopy.map((f) => path.basename(f)));
			}

			if (installedNames.length === 0) {
				throw new Error("No addons found in repository");
			}

			// Register in DB
			await this.scanInstalledAddons();

			// Update records with Git Metadata and correct version
			for (const addonName of installedNames) {
				// Skip internal ElvUI addons from being "Managed"
				if (
					addonName === "ElvUI_OptionsUI" ||
					addonName === "ElvUI_Libraries"
				) {
					this.dbManager.removeAddon(addonName);
					continue;
				}

				this.dbManager.updateAddon(addonName, {
					url: url,
					type: "github",
					last_updated: new Date().toISOString(),
				});
			}

			await fs.rm(tempDir, { recursive: true, force: true });
			return { success: true, installedAddons: installedNames };
		} catch (error) {
			logger.error("Manager", "Install failed", error);
			await fs.rm(tempDir, { recursive: true, force: true });
			return {
				success: false,
				installedAddons: [],
				error: error instanceof Error ? error.message : String(error),
			};
		}
	}

	public getAllAddons() {
		return this.dbManager.getAll();
	}

	public getAddon(folder: string) {
		return this.dbManager.getByFolder(folder);
	}

	public async scanInstalledAddons(
		specificFolders?: string[],
	): Promise<number> {
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
			return 0;
		}

		logger.log(
			"Manager",
			`Scanning for addons in ${addonsDir}...${specificFolders ? ` (Targeted: ${specificFolders.length})` : ""}`,
		);

		// If doing a full scan, cleanup ignored addons from DB if they exist
		if (!specificFolders) {
			// Cleanup ignored addons from DB if they exist
			// ElvUI_OptionsUI was the old name, now it seems to be ElvUI_Options and ElvUI_Libraries
			const IGNORED_FOLDERS = [
				"ElvUI_OptionsUI",
				"ElvUI_Options",
				"ElvUI_Libraries",
			];
			for (const ignored of IGNORED_FOLDERS) {
				if (this.dbManager.getByFolder(ignored)) {
					this.dbManager.removeAddon(ignored);
					logger.log("Manager", `Removed ignored addon from DB: ${ignored}`);
				}
			}
		}

		const tocGlob = new Bun.Glob("*/*.toc");
		let count = 0;

		for await (const file of tocGlob.scan({ cwd: addonsDir })) {
			// file is relative path like "ElvUI/ElvUI.toc"
			const folderName = path.dirname(file); // "ElvUI"

			// Skip if folderName is "." or empty
			if (!folderName || folderName === ".") continue;

			// Optimization: Skip if not in specific list
			if (specificFolders && !specificFolders.includes(folderName)) {
				continue;
			}

			// Filter out internal ElvUI folders that shouldn't be managed separately
			if (
				folderName === "ElvUI_OptionsUI" ||
				folderName === "ElvUI_Options" ||
				folderName === "ElvUI_Libraries"
			)
				continue;

			const fullPath = path.join(addonsDir, file);

			try {
				const content = await Bun.file(fullPath).text();

				// Extract Metadata
				const titleMatch = content.match(/^## Title:\s*(.*)/m);
				const versionMatch = content.match(/^## Version:\s*(.*)/m);
				const authorMatch = content.match(/^## Author:\s*(.*)/m);
				const interfaceMatch = content.match(/^## Interface:\s*(.*)/m);

				const title = titleMatch?.[1]?.trim() ?? folderName;
				// Remove color codes from title (e.g. |cff1784d1ElvUI|r)
				const cleanTitle = title.replace(/\|c[0-9a-fA-F]{8}(.*?)\|r/g, "$1");

				const version = versionMatch?.[1]?.trim() ?? null;
				const author = authorMatch?.[1]?.trim() ?? null;
				const gameInterface = interfaceMatch?.[1]?.trim() ?? null;

				// Detect if it's a Git repository
				const gitPath = path.join(addonsDir, folderName, ".git");
				let isGit = false;
				let gitHash: string | null = null;
				try {
					await fs.stat(gitPath);
					isGit = true;
					// If git, get hash
					gitHash = await GitClient.getCurrentCommit(
						path.join(addonsDir, folderName),
					);
				} catch {
					isGit = false;
				}

				// Determine final version: TOC Version > Git Hash
				const finalVersion =
					version || (isGit && gitHash ? gitHash : "Unknown");

				// Upsert into DB
				const existing = this.dbManager.getByFolder(folderName);

				if (existing) {
					const updates: Partial<AddonRecord> = {};
					let updated = false;

					// Auto-promote to GitHub type if .git folder exists
					if (isGit && existing.type !== "github") {
						updates.type = "github";
						updated = true;
					}

					// Update version/metadata if changed
					if (finalVersion && existing.version !== finalVersion) {
						updates.version = finalVersion;
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
					// Create new record
					this.dbManager.addAddon({
						name: cleanTitle,
						folder: folderName,
						version: finalVersion,
						author: author,
						interface: gameInterface,
						url: null, // We'll need to fetch this later or let install handle it
						type: isGit ? "github" : "manual",
						install_date: new Date().toISOString(),
						last_updated: new Date().toISOString(),
					});
					count++;
					logger.log(
						"Manager",
						`Discovered addon: ${cleanTitle} (${folderName})`,
					);
				}
			} catch (e) {
				logger.error("Manager", `Failed to parse ${file}`, e);
			}
		}

		logger.log("Manager", `Scan complete. Found ${count} new/updated addons.`);
		return count;
	}

	public updateAddonMetadata(folder: string, metadata: Partial<AddonRecord>) {
		this.dbManager.updateAddon(folder, metadata);
	}

	public async removeAddon(folder: string): Promise<boolean> {
		logger.log("Manager", `Removing addon: ${folder}`);

		// Get details before removal for Config sync
		const addon = this.dbManager.getByFolder(folder);

		// 1. Remove from DB
		this.dbManager.removeAddon(folder);

		// 2. Remove from Config (if applicable)
		if (addon) {
			this.configManager.removeRepository(addon.name);
		}

		// 3. Remove from Disk
		const addonsDir = this.configManager.get().destDir;
		const addonPath = path.join(addonsDir, folder);

		try {
			await fs.rm(addonPath, { recursive: true, force: true });
			return true;
		} catch (error) {
			logger.error(
				"Manager",
				`Failed to delete addon folder: ${folder}`,
				error,
			);
			return false;
		}
	}
}
