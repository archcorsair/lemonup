import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ConfigManager, REPO_TYPE } from "./config";
import { AddonManager } from "./manager";

// --- Mocks ---
// We need to mock these BEFORE importing AddonManager or inside the test file before usage
// But since we are in the same file, we can use mock.module
const mockGetRemoteCommit = mock();
const mockClone = mock();
const mockDownload = mock();
const mockUnzip = mock();

mock.module("./git", () => ({
	getRemoteCommit: mockGetRemoteCommit,
	clone: mockClone,
}));

mock.module("./downloader", () => ({
	download: mockDownload,
	unzip: mockUnzip,
}));

const TMP_BASE = path.join(os.tmpdir(), "lemonup-tests-manager");
const CONFIG_DIR = path.join(TMP_BASE, "config");
const DEST_DIR = path.join(TMP_BASE, "AddOns");

describe("AddonManager", () => {
	let configManager: ConfigManager;
	let manager: AddonManager;

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
	});

	test("checkUpdate should return true if versions differ", async () => {
		const repo = {
			name: "test-repo",
			type: REPO_TYPE.GITHUB,
			gitRemote: "http://git",
			branch: "main",
			folders: ["test"],
			installedVersion: "old-hash",
			downloadUrl: undefined,
		};

		mockGetRemoteCommit.mockResolvedValue("new-hash");

		const result = await manager.checkUpdate(repo);

		expect(result.updateAvailable).toBe(true);
		expect(result.remoteVersion).toBe("new-hash");
		expect(mockGetRemoteCommit).toHaveBeenCalledWith("http://git", "main");
	});

	test("updateRepository (GitHub) should clone and install", async () => {
		const repo = {
			name: "gh-repo",
			type: REPO_TYPE.GITHUB,
			gitRemote: "http://git",
			branch: "main",
			folders: ["FolderA"],
			installedVersion: "old-hash",
			downloadUrl: undefined,
		};

		mockGetRemoteCommit.mockResolvedValue("new-hash");
		mockClone.mockImplementation(async (_url, _branch, dest) => {
			// Fake the clone by creating the folder in dest
			const folderPath = path.join(dest, "FolderA");
			fs.mkdirSync(folderPath, { recursive: true });
			await Bun.write(path.join(folderPath, "file.txt"), "content");
			return true;
		});

		const result = await manager.updateRepository(
			repo,
			configManager.get(),
			TMP_BASE,
			false,
		);

		expect(result.success).toBe(true);
		expect(result.updated).toBe(true);
		expect(mockClone).toHaveBeenCalled();

		// Check if installed
		const installedFile = path.join(DEST_DIR, "FolderA", "file.txt");
		expect(await Bun.file(installedFile).exists()).toBe(true);

		// Check config update
		// Note: repo is not added to config in this test, so we don't check config update persistence here.
		// Detailed config update logic is tested in "updateRepository (GitHub) with repo in config"
	});

	// Fix for the above observation:
	test("updateRepository (GitHub) with repo in config", async () => {
		const repo = {
			name: "gh-repo-real",
			type: REPO_TYPE.GITHUB,
			gitRemote: "http://git",
			branch: "main",
			folders: ["FolderA"],
			installedVersion: "old-hash",
			downloadUrl: undefined,
		};
		configManager.set("repositories", [repo]);

		mockGetRemoteCommit.mockResolvedValue("new-hash");
		mockClone.mockImplementation(async (_url, _branch, dest) => {
			const folderPath = path.join(dest, "FolderA");
			fs.mkdirSync(folderPath, { recursive: true });
			return true;
		});

		const result = await manager.updateRepository(
			repo,
			configManager.get(),
			TMP_BASE,
			false,
		);

		expect(result.success).toBe(true);

		// Verify config updated
		const conf = configManager.get();
		const savedRepo = conf.repositories.find((r) => r.name === "gh-repo-real");
		expect(savedRepo?.installedVersion).toBe("new-hash");
	});

	test("updateRepository (TukUI) should download and install", async () => {
		const repo = {
			name: "tukui-repo",
			type: REPO_TYPE.TUKUI,
			gitRemote: "http://git-check", // used for version check
			branch: "main",
			folders: ["TukUI"],
			installedVersion: "old-hash",
			downloadUrl: "http://download",
		};
		configManager.set("repositories", [repo]);

		mockGetRemoteCommit.mockResolvedValue("new-hash");

		mockDownload.mockResolvedValue(true);
		mockUnzip.mockImplementation(async (_zipPath, dest) => {
			// Fake unzip
			const folderPath = path.join(dest, "TukUI");
			fs.mkdirSync(folderPath, { recursive: true });
			return true;
		});

		const result = await manager.updateRepository(
			repo,
			configManager.get(),
			TMP_BASE,
			false,
		);

		expect(result.success).toBe(true);
		expect(mockDownload).toHaveBeenCalled();
		expect(mockUnzip).toHaveBeenCalled();

		const installedFolder = path.join(DEST_DIR, "TukUI");
		expect(fs.existsSync(installedFolder)).toBe(true);
	});
});
