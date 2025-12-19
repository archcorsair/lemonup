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
import { ConfigManager } from "../config";
import { DatabaseManager } from "../db";
import * as Downloader from "../downloader";
import * as GitClient from "../git";
import { InstallFromUrlCommand } from "./InstallFromUrlCommand";
import { InstallTukUICommand } from "./InstallTukUICommand";
import { RemoveAddonCommand } from "./RemoveAddonCommand";
import { ScanCommand } from "./ScanCommand";
import { UpdateAddonCommand } from "./UpdateAddonCommand";

const TMP_BASE = path.join(os.tmpdir(), "lemonup-tests-commands");
const CONFIG_DIR = path.join(TMP_BASE, "config");
const DEST_DIR = path.join(TMP_BASE, "AddOns");

describe("Commands", () => {
	let configManager: ConfigManager;
	let dbManager: DatabaseManager;
	// biome-ignore lint/suspicious/noExplicitAny: test mock
	const mockContext: any = {
		emit: mock(),
	};

	beforeEach(() => {
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

		mockContext.emit.mockReset();

		if (fs.existsSync(TMP_BASE)) {
			fs.rmSync(TMP_BASE, { recursive: true, force: true });
		}
		fs.mkdirSync(CONFIG_DIR, { recursive: true });
		fs.mkdirSync(DEST_DIR, { recursive: true });

		configManager = new ConfigManager({ cwd: CONFIG_DIR });
		configManager.createDefaultConfig();
		configManager.set("destDir", DEST_DIR);

		dbManager = new DatabaseManager(CONFIG_DIR);
	});

	afterEach(() => {
		mock.restore();
		dbManager.close();
		if (fs.existsSync(TMP_BASE)) {
			try {
				fs.rmSync(TMP_BASE, { recursive: true, force: true });
			} catch (_) {}
		}
	});

	describe("InstallFromUrlCommand", () => {
		test("should install from github url", async () => {
			const url = "https://github.com/user/repo";

			spyOn(GitClient, "clone").mockImplementation(
				async (_url, _branch, dest) => {
					const folderPath = path.join(dest, "RepoAddon");
					fs.mkdirSync(folderPath, { recursive: true });
					await Bun.write(
						path.join(folderPath, "RepoAddon.toc"),
						"## Title: Repo Addon",
					);
					return true;
				},
			);

			const command = new InstallFromUrlCommand(dbManager, configManager, url);
			const result = await command.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.installedAddons).toContain("RepoAddon");
			expect(GitClient.clone).toHaveBeenCalled();
			expect(mockContext.emit).toHaveBeenCalledWith(
				"addon:install:complete",
				url,
			);

			const addon = dbManager.getByFolder("RepoAddon");
			expect(addon).toBeTruthy();
			expect(addon?.type).toBe("github");
			expect(addon?.url).toBe(url);
		});
	});

	describe("InstallTukUICommand", () => {
		test("should install ElvUI", async () => {
			const url = "http://elvui";
			const folder = "ElvUI";

			spyOn(Downloader, "download").mockResolvedValue(true);
			spyOn(Downloader, "unzip").mockImplementation(async (_zip, dest) => {
				const folderPath = path.join(dest, "ElvUI");
				fs.mkdirSync(folderPath, { recursive: true });
				await Bun.write(
					path.join(folderPath, "ElvUI.toc"),
					"## Title: ElvUI\n## Version: 1.0",
				);
				const libPath = path.join(dest, "ElvUI_Libraries");
				fs.mkdirSync(libPath, { recursive: true });
				return true;
			});

			const command = new InstallTukUICommand(
				dbManager,
				configManager,
				url,
				folder,
				["ElvUI_Libraries"],
			);
			const result = await command.execute(mockContext);

			expect(result).toBe(true);
			expect(Downloader.download).toHaveBeenCalled();
			expect(Downloader.unzip).toHaveBeenCalled();
			expect(mockContext.emit).toHaveBeenCalledWith(
				"addon:install:complete",
				"ElvUI",
			);

			expect(dbManager.getByFolder("ElvUI")).toBeTruthy();
		});
	});

	describe("RemoveAddonCommand", () => {
		test("should remove addon and folder", async () => {
			const folder = "ToBeRemoved";
			const addonDir = path.join(DEST_DIR, folder);
			fs.mkdirSync(addonDir, { recursive: true });

			dbManager.addAddon({
				name: "ToBeRemoved",
				folder: folder,
				version: "1.0",
				type: "manual",
				git_commit: null,
				author: null,
				interface: null,
				url: null,
				install_date: new Date().toISOString(),
				last_updated: new Date().toISOString(),
			});

			const command = new RemoveAddonCommand(dbManager, configManager, folder);
			const result = await command.execute(mockContext);

			expect(result).toBe(true);
			expect(fs.existsSync(addonDir)).toBe(false);
			expect(dbManager.getByFolder(folder)).toBeNull();
			expect(mockContext.emit).toHaveBeenCalledWith(
				"addon:remove:complete",
				folder,
			);
		});
	});

	describe("UpdateAddonCommand", () => {
		test("should update github addon", async () => {
			const folder = "UpdateMe";
			const addonDir = path.join(DEST_DIR, folder);
			fs.mkdirSync(addonDir, { recursive: true });

			// biome-ignore lint/suspicious/noExplicitAny: test data
			const addon: any = {
				name: "UpdateMe",
				folder: folder,
				type: "github",
				url: "http://git",
				version: "old",
				author: null,
				interface: null,
			};

			dbManager.addAddon({ ...addon, install_date: "", last_updated: "" });

			const newHash = "a1b2c3d4e5f678901234567890abcdef12345678";
			spyOn(GitClient, "getRemoteCommit").mockResolvedValue(newHash);
			spyOn(GitClient, "clone").mockImplementation(
				async (_url, _branch, dest) => {
					const folderPath = path.join(dest, folder);
					fs.mkdirSync(folderPath, { recursive: true });
					return true;
				},
			);

			const command = new UpdateAddonCommand(
				dbManager,
				configManager,
				addon,
				false,
			);
			const result = await command.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.updated).toBe(true);
			expect(GitClient.clone).toHaveBeenCalled();

			const updated = dbManager.getByFolder(folder);
			expect(updated?.git_commit).toBe(newHash);
		});
	});

	describe("ScanCommand", () => {
		test("should scan existing addons", async () => {
			const folder = "ExistingAddon";
			const addonDir = path.join(DEST_DIR, folder);
			fs.mkdirSync(addonDir, { recursive: true });

			const tocContent =
				"## Title: Existing Addon\n## Version: 1.2.3\n## Author: Me\n";
			await Bun.write(path.join(addonDir, `${folder}.toc`), tocContent);

			const command = new ScanCommand(dbManager, configManager);
			const count = await command.execute(mockContext);

			expect(count).toBeGreaterThan(0);
			expect(mockContext.emit).toHaveBeenCalledWith(
				"scan:complete",
				expect.any(Number),
			);

			const addon = dbManager.getByFolder(folder);
			expect(addon).toBeTruthy();
			expect(addon?.name).toBe("Existing Addon");
			expect(addon?.version).toBe("1.2.3");
			expect(addon?.author).toBe("Me");
			expect(addon?.type).toBe("manual");
		});
	});
});
