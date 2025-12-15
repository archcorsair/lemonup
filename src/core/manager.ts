import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { $ } from "bun";
import { type Config, ConfigManager, type Repository } from "./config";
import * as Downloader from "./downloader";
import * as GitClient from "./git";

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

	public async updateRepository(
		repo: Repository,
		config: Config,
		tempBaseDir: string,
		force: boolean,
		dryRun = false,
		onProgress?: (status: string) => void,
	): Promise<UpdateResult> {
		const { name, gitRemote, branch, installedVersion } = repo;

		// 1. Check Remote Version
		onProgress?.("checking");
		// Note: Both Github and TukUI (in this setup) use git ls-remote to check versions.
		if (!gitRemote) {
			return {
				repoName: name,
				success: false,
				updated: false,
				error: "Missing gitRemote",
			};
		}

		// In dry-run, we still check the remote to see if an update IS available.
		// Use a random delay if dryRun to simulate network if we wanted, but real network check is better.
		// However, to make it "Simulated" for TUI testing without hitting API limits, maybe we should mock it?
		// User asked: "Implement a dry-run (simulated run) to be able to test the TUI output."
		// This suggests they want to see the UI animations. Real network calls are fast/slow varying.
		// Let's do Real Check -> Dry Install.

		const remoteHash = await GitClient.getRemoteCommit(gitRemote, branch);
		if (!remoteHash) {
			return {
				repoName: name,
				success: false,
				updated: false,
				error: "Failed to get remote hash",
			};
		}

		// 2. Compare Versions
		if (!force && installedVersion === remoteHash) {
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
			// Simulate install delay for TUI visualization
			await new Promise((resolve) => setTimeout(resolve, 2000));
			return {
				repoName: name,
				success: true,
				updated: true,
				message: `[Dry Run] Would update to ${remoteHash.substring(0, 7)}`,
			};
		}

		let success = false;
		// Separate directory for this specific repo's operations
		const repoTempDir = path.join(tempBaseDir, name);
		// clean it before use
		await fs.rm(repoTempDir, { recursive: true, force: true });

		if (repo.type === "tukui") {
			success = await this.installTukUI(repo, config, repoTempDir);
		} else if (repo.type === "github") {
			success = await this.installGithub(repo, config, repoTempDir);
		} else {
			return {
				repoName: name,
				success: false,
				updated: false,
				error: `Unknown type: ${repo.type}`,
			};
		}

		if (success) {
			// 4. Update Config
			this.configManager.updateRepository(name, {
				installedVersion: remoteHash,
			});
			return {
				repoName: name,
				success: true,
				updated: true,
				message: `Updated to ${remoteHash.substring(0, 7)}`,
			};
		} else {
			return {
				repoName: name,
				success: false,
				updated: false,
				error: "Installation failed",
			};
		}
	}

	private async installTukUI(
		repo: Repository,
		config: Config,
		tempDir: string,
	): Promise<boolean> {
		if (!repo.downloadUrl) return false;

		await fs.mkdir(tempDir, { recursive: true });
		const zipPath = path.join(tempDir, "addon.zip");

		// Download
		if (!(await Downloader.download(repo.downloadUrl, zipPath))) {
			return false;
		}

		// Extract
		const extractPath = path.join(tempDir, "extract");
		await fs.mkdir(extractPath, { recursive: true });
		if (!(await Downloader.unzip(zipPath, extractPath))) {
			return false;
		}

		// Install (Copy)
		return await this.copyFolders(extractPath, config.destDir, repo.folders);
	}

	private async installGithub(
		repo: Repository,
		config: Config,
		tempDir: string,
	): Promise<boolean> {
		if (!repo.gitRemote) return false;

		// Clone directly to tempDir
		if (!(await GitClient.clone(repo.gitRemote, tempDir, repo.branch))) {
			return false;
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
			for (const folder of folders) {
				const source = path.join(sourceBase, folder);
				const dest = path.join(destBase, folder);

				// Ensure source exists
				try {
					await fs.access(source);
				} catch {
					console.error(`Source folder not found: ${source}`);
					return false;
				}

				// Copy recursively using Bun shell for simplicity/power
				// cp -r -f source dest
				// Note: Bun.$ throws on error by default so we catch it
				await $`cp -r -f ${source} ${dest}`;
			}
			return true;
		} catch (error) {
			console.error("Copy failed:", error);
			return false;
		}
	}
}
