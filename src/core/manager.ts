import { EventEmitter } from "node:events";
import fs from "node:fs/promises";
import path from "node:path";
import {
	InstallFromUrlCommand,
	type InstallFromUrlResult,
} from "./commands/InstallFromUrlCommand";
import { InstallTukUICommand } from "./commands/InstallTukUICommand";
import { RemoveAddonCommand } from "./commands/RemoveAddonCommand";
import { ScanCommand } from "./commands/ScanCommand";
import type { Command, CommandContext } from "./commands/types";
import {
	UpdateAddonCommand,
	type UpdateAddonResult,
} from "./commands/UpdateAddonCommand";
import { type Config, ConfigManager, type Repository } from "./config";
import { type AddonRecord, DatabaseManager } from "./db";
import type { AddonManagerEvents } from "./events";
import * as GitClient from "./git";
import { logger } from "./logger";

export interface UpdateResult {
	repoName: string;
	success: boolean;
	updated: boolean;
	message?: string;
	error?: string;
}

export class AddonManager extends EventEmitter {
	private configManager: ConfigManager;
	private dbManager: DatabaseManager;

	constructor(configManager?: ConfigManager) {
		super();
		this.configManager = configManager || new ConfigManager();
		const configDir = path.dirname(this.configManager.path);
		this.dbManager = new DatabaseManager(configDir);

		this.migrateConfig();
	}

	public override emit<K extends keyof AddonManagerEvents>(
		event: K,
		...args: AddonManagerEvents[K]
	): boolean {
		return super.emit(event, ...args);
	}

	private async executeCommand<T>(command: Command<T>): Promise<T> {
		const context: CommandContext = {
			emit: (event, ...args) => this.emit(event, ...args),
		};
		return await command.execute(context);
	}

	public getConfig(): Config {
		return this.configManager.get();
	}

	public setConfigValue<K extends keyof Config>(key: K, value: Config[K]) {
		this.configManager.set(key, value);
	}

	public close() {
		this.dbManager.close();
	}

	private migrateConfig() {
		const config = this.configManager.get();

		if (config.migrated_to_db) {
			return;
		}

		if (config.repositories && config.repositories.length > 0) {
			logger.log(
				"Manager",
				"Migrating repositories from config to database...",
			);
			for (const repo of config.repositories) {
				const folder = repo.folders[0];
				if (!folder) continue;

				if (this.dbManager.getByFolder(folder)) continue;

				this.dbManager.addAddon({
					name: repo.name,
					folder: folder,
					version: repo.installedVersion,
					git_commit: null,
					author: null,
					interface: null,
					url: repo.gitRemote || repo.downloadUrl || null,
					type: repo.type as "github" | "tukui",
					install_date: new Date().toISOString(),
					last_updated: new Date().toISOString(),
				});
				logger.log("Manager", `Migrated ${repo.name}`);
			}
		}

		this.configManager.set("migrated_to_db", true);
	}

	public async updateAll(force = false): Promise<UpdateAddonResult[]> {
		const addons = this.dbManager.getAll();
		const results: UpdateAddonResult[] = [];

		for (const addon of addons) {
			if (addon.type === "manual") continue;

			try {
				const result = await this.updateAddon(addon, force);
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

		return results;
	}

	public async checkUpdate(addon: AddonRecord): Promise<{
		updateAvailable: boolean;
		remoteVersion: string;
		error?: string;
	}> {
		if (!addon.url) {
			return {
				updateAvailable: false,
				remoteVersion: "",
				error: "Missing URL",
			};
		}

		if (addon.type === "github") {
			const branch = "main";
			const remoteHash = await GitClient.getRemoteCommit(addon.url, branch);
			if (!remoteHash) {
				return {
					updateAvailable: false,
					remoteVersion: "",
					error: "Failed to get remote hash",
				};
			}

			// Compare with stored git_commit if available, otherwise fallback to version (legacy behavior)
			const localHash = addon.git_commit || addon.version;
			const isUpdate = localHash !== remoteHash;

			return { updateAvailable: isUpdate, remoteVersion: remoteHash };
		}

		if (addon.type === "tukui") {
			if (addon.name === "ElvUI" || addon.folder === "ElvUI") {
				// Try to extract git hash from TOC version (e.g. v13.08-1-g123abc)
				const hashMatch = addon.version?.match(/-g([a-f0-9]+)/);
				const localHashFromVer = hashMatch ? hashMatch[1] : null;
				// Prefer stored git_commit
				const localHash = addon.git_commit || localHashFromVer;

				const remoteHash = await GitClient.getRemoteCommit(
					"https://github.com/tukui-org/ElvUI",
					"main",
				);

				if (remoteHash && localHash) {
					// Check if remote full hash starts with local short hash
					if (
						remoteHash.startsWith(localHash) ||
						localHash.startsWith(remoteHash)
					) {
						return {
							updateAvailable: false,
							remoteVersion: remoteHash,
						};
					}
					return {
						updateAvailable: true,
						remoteVersion: remoteHash,
					};
				}

				if (remoteHash) {
					// If we have remote but no local hash, assume update needed
					// UNLESS we just installed it?
					// If git_commit is null, it means we scanned a manual install or TOC version with no hash.
					// We can't verify.
					return {
						updateAvailable: true,
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
		force: boolean,
	): Promise<UpdateAddonResult> {
		const command = new UpdateAddonCommand(
			this.dbManager,
			this.configManager,
			addon,
			force,
		);
		return await this.executeCommand(command);
	}

	public async installFromUrl(url: string): Promise<InstallFromUrlResult> {
		const command = new InstallFromUrlCommand(
			this.dbManager,
			this.configManager,
			url,
		);
		return await this.executeCommand(command);
	}

	public async installTukUI(
		url: string,
		addonFolder: string,
		subFolders: string[] = [],
	): Promise<boolean> {
		const command = new InstallTukUICommand(
			this.dbManager,
			this.configManager,
			url,
			addonFolder,
			subFolders,
		);
		return await this.executeCommand(command);
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
		const command = new ScanCommand(
			this.dbManager,
			this.configManager,
			specificFolders,
		);
		return await this.executeCommand(command);
	}

	public updateAddonMetadata(folder: string, metadata: Partial<AddonRecord>) {
		this.dbManager.updateAddon(folder, metadata);
	}

	public async removeAddon(folder: string): Promise<boolean> {
		const command = new RemoveAddonCommand(
			this.dbManager,
			this.configManager,
			folder,
		);
		return await this.executeCommand(command);
	}

	public isAlreadyInstalled(urlOrFolder: string): boolean {
		const addons = this.dbManager.getAll();
		const clean = (u: string) =>
			u
				.replace(/\/$/, "")
				.replace(/\.git$/, "")
				.toLowerCase();

		const target = clean(urlOrFolder);

		// Check by folder name or URL
		return addons.some(
			(a) => clean(a.folder) === target || (a.url && clean(a.url) === target),
		);
	}
}
