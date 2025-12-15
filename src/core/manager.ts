import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
	type Config,
	ConfigManager,
	REPO_TYPE,
	type Repository,
} from "./config";
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

	constructor(configManager?: ConfigManager) {
		this.configManager = configManager || new ConfigManager();
	}

	public getConfig(): Config {
		return this.configManager.get();
	}

	/**
	 * Main entry point to update all repositories.
	 */
	public async updateAll(force = false): Promise<UpdateResult[]> {
		const config = this.configManager.get();
		const results: UpdateResult[] = [];

		// Ensure temp dir exists
		const tempDir = path.join(os.tmpdir(), "lemonup");
		await fs.mkdir(tempDir, { recursive: true });

		for (const repo of config.repositories) {
			try {
				const result = await this.updateRepository(
					repo,
					config,
					tempDir,
					force,
				);
				results.push(result);
			} catch (error) {
				results.push({
					repoName: repo.name,
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

	public async checkUpdate(repo: Repository): Promise<{
		updateAvailable: boolean;
		remoteVersion: string;
		error?: string;
	}> {
		const { name, gitRemote, branch, installedVersion } = repo;

		if (!gitRemote) {
			return {
				updateAvailable: false,
				remoteVersion: "",
				error: "Missing gitRemote",
			};
		}

		const remoteHash = await GitClient.getRemoteCommit(gitRemote, branch);
		if (!remoteHash) {
			return {
				updateAvailable: false,
				remoteVersion: "",
				error: "Failed to get remote hash",
			};
		}

		const result = {
			updateAvailable: installedVersion !== remoteHash,
			remoteVersion: remoteHash,
		};

		return result;
	}

	public async updateRepository(
		repo: Repository,
		config: Config,
		tempBaseDir: string,
		force: boolean,
		dryRun = false,
		onProgress?: (status: string) => void,
	): Promise<UpdateResult> {
		const { name } = repo;
		logger.log(
			"Manager",
			`Starting updateRepository for ${name}. Force=${force}, DryRun=${dryRun}`,
		);

		// 1. Check Remote Version
		onProgress?.("checking");
		// NOTE: Both Github and TukUI use git ls-remote to check versions.
		if (!repo.gitRemote) {
			return {
				repoName: name,
				success: false,
				updated: false,
				error: "Missing gitRemote",
			};
		}

		const check = await this.checkUpdate(repo);
		if (check.error) {
			return {
				repoName: name,
				success: false,
				updated: false,
				error: check.error,
			};
		}

		const remoteHash = check.remoteVersion;

		// 2. Compare Versions
		if (!force && !check.updateAvailable) {
			return {
				repoName: name,
				success: true,
				updated: false,
				message: "Up to date",
			};
		}

		// 3. Perform Update
		onProgress?.("downloading");

		if (dryRun) {
			await new Promise((resolve) => setTimeout(resolve, 2000));
			return {
				repoName: name,
				success: true,
				updated: true,
				message: `[Dry Run] Would update to ${remoteHash.substring(0, 7)}`,
			};
		}

		// Separate directory for this specific repo's operations
		const repoTempDir = path.join(tempBaseDir, name);
		// clean it before use
		await fs.rm(repoTempDir, { recursive: true, force: true });

		try {
			if (repo.type === REPO_TYPE.TUKUI) {
				await this.installTukUI(repo, config, repoTempDir);
			} else if (repo.type === REPO_TYPE.GITHUB) {
				await this.installGithub(repo, config, repoTempDir);
			} else {
				throw new Error(`Unknown type: ${repo.type}`);
			}

			// 4. Update Config
			logger.log("Manager", `Updating config for ${name} to ${remoteHash}`);
			this.configManager.updateRepository(name, {
				installedVersion: remoteHash,
			});
			return {
				repoName: name,
				success: true,
				updated: true,
				message: `Updated to ${remoteHash.substring(0, 7)}`,
			};
		} catch (err: unknown) {
			logger.error("Manager", `Error updating ${name}`, err);
			// If we catch here, we can return the nice UpdateResult with error
			// preventing the caller from needing try/catch if we prefer.
			// But `UpdateScreen` expects to handle bubbles OR result.success=false.
			// Let's return the error result here so `UpdateScreen` uses it.
			return {
				repoName: name,
				success: false,
				updated: false,
				error: err instanceof Error ? err.message : String(err),
			};
		}
	}

	private async installTukUI(
		repo: Repository,
		config: Config,
		tempDir: string,
	): Promise<boolean> {
		if (!repo.downloadUrl) throw new Error("Missing downloadUrl");

		await fs.mkdir(tempDir, { recursive: true });
		const zipPath = path.join(tempDir, "addon.zip");

		// Download
		if (!(await Downloader.download(repo.downloadUrl, zipPath))) {
			throw new Error("Download failed");
		}

		// Extract
		const extractPath = path.join(tempDir, "extract");
		await fs.mkdir(extractPath, { recursive: true });
		if (!(await Downloader.unzip(zipPath, extractPath))) {
			throw new Error("Unzip failed");
		}

		// Install (Copy)
		return await this.copyFolders(extractPath, config.destDir, repo.folders);
	}

	private async installGithub(
		repo: Repository,
		config: Config,
		tempDir: string,
	): Promise<boolean> {
		if (!repo.gitRemote) throw new Error("Missing gitRemote");

		// Clone directly to tempDir
		if (!(await GitClient.clone(repo.gitRemote, repo.branch, tempDir))) {
			throw new Error("Git Clone failed");
		}

		// Install (Copy)
		return await this.copyFolders(tempDir, config.destDir, repo.folders);
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
}
