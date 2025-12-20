import {
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ConfigManager } from "@/core/config";
import * as Downloader from "@/core/downloader";
import * as GitClient from "@/core/git";
import * as TukUI from "@/core/tukui";
import type { AddonManager as AddonManagerType } from "@/core/manager";

// Import AddonManager
const { AddonManager } = await import("@/core/manager");

const TMP_BASE = path.join(os.tmpdir(), "lemonup-tests-manager");
const CONFIG_DIR = path.join(TMP_BASE, "config");
const DEST_DIR = path.join(TMP_BASE, "AddOns");

describe("AddonManager", () => {
	let configManager: ConfigManager;
	let manager: AddonManagerType;

	beforeEach(() => {
		// Setup Spies
		spyOn(GitClient, "getRemoteCommit").mockImplementation(() =>
			Promise.resolve("hash"),
		);
		spyOn(GitClient, "clone").mockImplementation(() => Promise.resolve(true));
		spyOn(GitClient, "getCurrentCommit").mockImplementation(() =>
			Promise.resolve("hash"),
		);

		spyOn(Downloader, "download").mockImplementation(() =>
			Promise.resolve(true),
		);
		spyOn(Downloader, "unzip").mockImplementation(() => Promise.resolve(true));

		// Setup FS
		if (fs.existsSync(TMP_BASE)) {
			fs.rmSync(TMP_BASE, { recursive: true, force: true });
		}
		fs.mkdirSync(CONFIG_DIR, { recursive: true });
		fs.mkdirSync(DEST_DIR, { recursive: true });

		// Setup Config
		configManager = new ConfigManager({ cwd: CONFIG_DIR });
		configManager.createDefaultConfig();
		configManager.set("destDir", DEST_DIR);

		manager = new AddonManager(configManager);
	});

	afterEach(() => {
		mock.restore();
		if (manager) manager.close();
		if (fs.existsSync(TMP_BASE)) {
			try {
				fs.rmSync(TMP_BASE, { recursive: true, force: true });
			} catch (_) {}
		}
	});

	test("checkUpdate should return true if versions differ (fallback)", async () => {
		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "test-repo",
			type: "github",
			url: "http://git",
			version: "old-hash",
			git_commit: null,
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue("new-hash");

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(true);
		expect(result.remoteVersion).toBe("new-hash");
		expect(GitClient.getRemoteCommit).toHaveBeenCalledWith(
			"http://git",
			"main",
		);
	});

	test("checkUpdate should use git_commit if available", async () => {
		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "test-repo",
			type: "github",
			url: "http://git",
			version: "v1.0.0",
			git_commit: "old-hash",
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue("new-hash");

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(true);
		expect(result.remoteVersion).toBe("new-hash");
	});

	test("checkUpdate should return false if git_commit matches remote", async () => {
		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "test-repo",
			type: "github",
			url: "http://git",
			version: "v1.0.0",
			git_commit: "same-hash",
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue("same-hash");

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(false);
	});

	test("updateAddon (GitHub) should clone and install", async () => {
		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "gh-repo",
			folder: "FolderA",
			type: "github",
			url: "http://git",
			version: "old-hash",
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue("new-hash");
		spyOn(GitClient, "clone").mockImplementation(
			async (_url, _branch, dest) => {
				const folderPath = path.join(dest, "FolderA");
				fs.mkdirSync(folderPath, { recursive: true });
				await Bun.write(path.join(folderPath, "file.txt"), "content");
				await Bun.write(
					path.join(folderPath, "FolderA.toc"),
					"## Title: FolderA",
				);
				return true;
			},
		);

		const result = await manager.updateAddon(addon, false);

		expect(result.success).toBe(true);
		expect(result.updated).toBe(true);
		expect(GitClient.clone).toHaveBeenCalled();

		const installedFile = path.join(DEST_DIR, "FolderA", "file.txt");
		expect(await Bun.file(installedFile).exists()).toBe(true);
	});

	test("updateAddon (TukUI) should download and install", async () => {
		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "TukUI",
			folder: "TukUI",
			type: "tukui",
			url: "http://download",
			version: "old-version",
		};

		spyOn(TukUI, "getAddonDetails").mockResolvedValue({
			id: -1,
			slug: "tukui",
			name: "Tukui",
			version: "new-version",
			url: "http://download",
			author: "Tukz",
			directories: ["Tukui"],
		} as any);

		spyOn(Downloader, "download").mockResolvedValue(true);
		spyOn(Downloader, "unzip").mockImplementation(async (_zipPath, dest) => {
			const folderPath = path.join(dest, "TukUI");
			fs.mkdirSync(folderPath, { recursive: true });
			await Bun.write(
				path.join(folderPath, "TukUI.toc"),
				"## Title: TukUI",
			);
			return true;
		});

		const result = await manager.updateAddon(addon, false);

		expect(result.success).toBe(true);
		expect(Downloader.download).toHaveBeenCalled();
		expect(Downloader.unzip).toHaveBeenCalled();

		const installedFolder = path.join(DEST_DIR, "TukUI");
		expect(fs.existsSync(installedFolder)).toBe(true);
	});

	test("scanInstalledAddons should find and register specific addon", async () => {
		const addonDir = path.join(DEST_DIR, "MyAddon");
		fs.mkdirSync(addonDir, { recursive: true });
		const tocContent = `
## Title: My Addon
## Version: 1.2.3
## Author: Me
## Interface: 110000
`;
		await Bun.write(path.join(addonDir, "MyAddon.toc"), tocContent);

		const count = await manager.scanInstalledAddons();
		expect(count).toBe(1);
	});

	test("isAlreadyInstalled should detect addon by folder or URL", async () => {
		const addonDir = path.join(DEST_DIR, "ExistingAddon");
		fs.mkdirSync(addonDir, { recursive: true });
		await Bun.write(
			path.join(addonDir, "ExistingAddon.toc"),
			"## Title: Existing",
		);

		await manager.scanInstalledAddons();

		manager.updateAddonMetadata("ExistingAddon", {
			url: "https://github.com/user/existing.git",
		});

		expect(manager.isAlreadyInstalled("ExistingAddon")).toBe(true);
		expect(manager.isAlreadyInstalled("existingaddon")).toBe(true);
		expect(manager.isAlreadyInstalled("https://github.com/user/existing")).toBe(
			true,
		);
		expect(
			manager.isAlreadyInstalled("https://github.com/user/existing.git/"),
		).toBe(true);
		expect(manager.isAlreadyInstalled("NonExistent")).toBe(false);
	});
});
