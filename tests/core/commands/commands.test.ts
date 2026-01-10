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
import { DatabaseManager } from "@/core/db";
import * as Downloader from "@/core/downloader";
import * as GitClient from "@/core/git";
import { InstallFromUrlCommand } from "@/core/commands/InstallFromUrlCommand";
import { InstallTukUICommand } from "@/core/commands/InstallTukUICommand";
import { RemoveAddonCommand } from "@/core/commands/RemoveAddonCommand";
import { isPathConfigured } from "@/core/paths";
import * as WoWInterface from "@/core/wowinterface";
import * as TukUI from "@/core/tukui";
import { ScanCommand } from "@/core/commands/ScanCommand";
import { UpdateAddonCommand } from "@/core/commands/UpdateAddonCommand";

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
		spyOn(GitClient, "clone").mockImplementation(async (_url, _branch, dir) => {
			fs.mkdirSync(dir, { recursive: true });
			const repoName = "RepoAddon";
			const addonDir = path.join(dir, repoName);
			fs.mkdirSync(addonDir, { recursive: true });
			fs.writeFileSync(
				path.join(addonDir, `${repoName}.toc`),
				`## Title: ${repoName}`,
			);
			return true;
		});
		spyOn(GitClient, "getCurrentCommit").mockImplementation(() =>
			Promise.resolve("hash"),
		);

		spyOn(Downloader, "download").mockImplementation((_url, dest) => {
			if (dest) {
				fs.writeFileSync(dest, "dummy zip content");
			}
			return Promise.resolve(true);
		});
		spyOn(Downloader, "unzip").mockImplementation(() => Promise.resolve(true));

		mockContext.emit.mockReset();

		try {
			if (fs.existsSync(TMP_BASE)) {
				fs.rmSync(TMP_BASE, { recursive: true, force: true });
			}
		} catch (err: any) {
			if (err?.code !== "EBUSY") {
				throw err;
			}
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

			const command = new InstallFromUrlCommand(dbManager, configManager, url);
			const result = await command.execute(mockContext);

			if (!result.success) {
				console.error("Install failed:", result.error);
			}

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

		test("should install from wowinterface url", async () => {
			const url = "https://wowinterface.com/downloads/info5108-Clique.html";
			const mockDetails = {
				UID: "5108",
				UIName: "Clique",
				UIVersion: "4.2.11",
				UIDownload: "http://download/clique.zip",
				UIAuthorName: "Cladhaire",
				UIFileName: "clique.zip",
			};

			spyOn(WoWInterface, "getAddonDetails").mockResolvedValue({
				success: true,
				details: mockDetails,
			});
			spyOn(Downloader, "unzip").mockImplementation(async (_zip, dest) => {
				const folderPath = path.join(dest, "Clique");
				fs.mkdirSync(folderPath, { recursive: true });
				await Bun.write(
					path.join(folderPath, "Clique.toc"),
					"## Title: Clique",
				);
				return true;
			});

			const command = new InstallFromUrlCommand(dbManager, configManager, url);
			const result = await command.execute(mockContext);

			if (!result.success) {
				console.error("WowInstall Failed:", result.error);
			}

			expect(result.success).toBe(true);
			expect(result.installedAddons).toContain("Clique");
			expect(WoWInterface.getAddonDetails).toHaveBeenCalledWith("5108");
			expect(dbManager.getByFolder("Clique")?.type).toBe("wowinterface");
		});
	});

	describe("InstallTukUICommand", () => {
		test("should install ElvUI", async () => {
			const url = "http://elvui";
			const folder = "ElvUI";

			spyOn(TukUI, "getAddonDetails").mockResolvedValue({
				id: -1,
				slug: "elvui",
				name: "ElvUI",
				version: "1.0",
				url: "http://elvui",
				author: "Tukz",
				directories: ["ElvUI"],
			} as any);

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
				ownedFolders: [],
				kind: "addon",
				kindOverride: false,
				flavor: "retail",
				requiredDeps: [],
				optionalDeps: [],
				embeddedLibs: [],
				git_commit: null,
				author: null,
				interface: null,
				url: null,
				install_date: new Date().toISOString(),
				    last_updated: new Date().toISOString(),
				    last_checked: null,
				    remote_version: null,
				   });
			const command = new RemoveAddonCommand(dbManager, configManager, folder, false);
			const result = await command.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.removedFolders).toContain(folder);
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
					await Bun.write(
						path.join(folderPath, `${folder}.toc`),
						`## Title: ${folder}`,
					);
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

		test("should update wowinterface addon", async () => {
			const folder = "WoWAddon";
			const addonDir = path.join(DEST_DIR, folder);
			fs.mkdirSync(addonDir, { recursive: true });

			// biome-ignore lint/suspicious/noExplicitAny: test data
			const addon: any = {
				name: "WoWAddon",
				folder: folder,
				type: "wowinterface",
				url: "https://wowinterface.com/downloads/info123-WoWAddon.html",
				version: "1.0",
				author: null,
				interface: null,
			};

			dbManager.addAddon({ ...addon, install_date: "", last_updated: "" });

			const mockDetails = {
				UID: "123",
				UIName: "WoWAddon",
				UIVersion: "1.1",
				UIDownload: "http://download/wowaddon.zip",
				UIAuthorName: "Author",
				UIFileName: "wowaddon.zip",
			};

			spyOn(WoWInterface, "getAddonDetails").mockResolvedValue({
				success: true,
				details: mockDetails,
			});
			spyOn(Downloader, "download").mockResolvedValue(true);
			spyOn(Downloader, "unzip").mockImplementation(async (_zip, dest) => {
				const folderPath = path.join(dest, folder);
				fs.mkdirSync(folderPath, { recursive: true });
				await Bun.write(
					path.join(folderPath, `${folder}.toc`),
					`## Title: ${folder}`,
				);
				return true;
			});

			const command = new UpdateAddonCommand(
				dbManager,
				configManager,
				addon,
				false,
			);
			const result = await command.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.updated).toBe(true);
			expect(WoWInterface.getAddonDetails).toHaveBeenCalled();
			expect(Downloader.download).toHaveBeenCalled();

			const updated = dbManager.getByFolder(folder);
			expect(updated?.version).toBe("1.1");
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
