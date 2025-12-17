import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ConfigManager } from "./config";
import type { AddonManager as AddonManagerType } from "./manager";

// --- Mocks ---
const mockGetRemoteCommit = mock();
const mockClone = mock();
const mockDownload = mock();
const mockUnzip = mock();

mock.module("./git", () => ({
	getRemoteCommit: mockGetRemoteCommit,
	clone: mockClone,
	getCurrentCommit: mock(() => Promise.resolve("hash")),
}));

mock.module("./downloader", () => ({
	download: mockDownload,
	unzip: mockUnzip,
}));

// Import AddonManager dynamically
const { AddonManager } = await import("./manager");

const TMP_BASE = path.join(os.tmpdir(), "lemonup-tests-manager");
const CONFIG_DIR = path.join(TMP_BASE, "config");
const DEST_DIR = path.join(TMP_BASE, "AddOns");

describe("AddonManager", () => {
	let configManager: ConfigManager;
	let manager: AddonManagerType;

	beforeEach(() => {
		// Clear mocks
		mockGetRemoteCommit.mockReset();
		mockClone.mockReset();
		mockDownload.mockReset();
		mockUnzip.mockReset();

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
		// Give SQLite a moment (sometimes needed on windows)
		// await new Promise(r => setTimeout(r, 10));
		if (fs.existsSync(TMP_BASE)) {
			try {
				fs.rmSync(TMP_BASE, { recursive: true, force: true });
			} catch (_) {}
		}
	});

	test("checkUpdate should return true if versions differ", async () => {
		const addon: any = {
			name: "test-repo",
			type: "github",
			url: "http://git",
			version: "old-hash",
		};

		mockGetRemoteCommit.mockResolvedValue("new-hash");

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(true);
		expect(result.remoteVersion).toBe("new-hash");
		expect(mockGetRemoteCommit).toHaveBeenCalledWith("http://git", "main");
	});

	test("updateAddon (GitHub) should clone and install", async () => {
		const addon: any = {
			name: "gh-repo",
			folder: "FolderA",
			type: "github",
			url: "http://git",
			version: "old-hash",
		};

		mockGetRemoteCommit.mockResolvedValue("new-hash");
		mockClone.mockImplementation(async (_url, _branch, dest) => {
			// Fake the clone by creating the folder in dest
			const folderPath = path.join(dest, "FolderA");
			fs.mkdirSync(folderPath, { recursive: true });
			await Bun.write(path.join(folderPath, "file.txt"), "content");
			return true;
		});

		const result = await manager.updateAddon(
			addon,
			false,
		);

		expect(result.success).toBe(true);
		expect(result.updated).toBe(true);
		expect(mockClone).toHaveBeenCalled();

		// Check if installed
		const installedFile = path.join(DEST_DIR, "FolderA", "file.txt");
		expect(await Bun.file(installedFile).exists()).toBe(true);
	});

	test("updateAddon (TukUI) should download and install", async () => {
		const addon: any = {
			name: "tukui-repo",
			folder: "TukUI",
			type: "tukui",
			url: "http://download",
			version: "old-hash",
		};

		mockDownload.mockResolvedValue(true);
		mockUnzip.mockImplementation(async (_zipPath, dest) => {
			// Fake unzip
			const folderPath = path.join(dest, "TukUI");
			fs.mkdirSync(folderPath, { recursive: true });
			return true;
		});

		const result = await manager.updateAddon(
			addon,
			false,
		);

		expect(result.success).toBe(true);
		expect(mockDownload).toHaveBeenCalled();
		expect(mockUnzip).toHaveBeenCalled();

		const installedFolder = path.join(DEST_DIR, "TukUI");
		expect(fs.existsSync(installedFolder)).toBe(true);
	});

	test("scanInstalledAddons should find and register specific addon", async () => {
		// Create a fake addon on disk
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
});
