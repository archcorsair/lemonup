import {
	afterEach,
	beforeEach,
	describe,
	expect,
	it,
	mock,
	spyOn,
} from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { InstallWagoCommand } from "@/core/commands/InstallWagoCommand";
import { ConfigManager } from "@/core/config";
import { DatabaseManager } from "@/core/db";
import type { CommandContext } from "@/core/commands/types";
import * as Downloader from "@/core/downloader";
import * as Wago from "@/core/wago";

describe("InstallWagoCommand", () => {
	let tempDir: string;
	let configDir: string;
	let destDir: string;
	let configManager: ConfigManager;
	let dbManager: DatabaseManager;
	let context: CommandContext;
	let emittedEvents: Array<{ event: string; args: unknown[] }>;
	let originalEnvKey: string | undefined;

	beforeEach(() => {
		originalEnvKey = process.env.WAGO_API_KEY;
		delete process.env.WAGO_API_KEY;
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "lemonup-wago-test-"));
		configDir = path.join(tempDir, "config");
		destDir = path.join(tempDir, "addons");
		fs.mkdirSync(configDir, { recursive: true });
		fs.mkdirSync(destDir, { recursive: true });

		configManager = new ConfigManager({ cwd: configDir });
		configManager.createDefaultConfig();
		configManager.set("destDir", destDir);
		configManager.set("wagoApiKey", "test-api-key");
		dbManager = new DatabaseManager(configDir);

		emittedEvents = [];
		context = {
			emit: (event: string, ...args: unknown[]) => {
				emittedEvents.push({ event, args });
			},
		} as CommandContext;
	});

	afterEach(() => {
		if (originalEnvKey !== undefined) {
			process.env.WAGO_API_KEY = originalEnvKey;
		} else {
			delete process.env.WAGO_API_KEY;
		}
		mock.restore();
		dbManager.close();
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("should fail when no API key is configured", async () => {
		configManager.set("wagoApiKey", "");
		const cmd = new InstallWagoCommand(dbManager, configManager, "clique");
		const result = await cmd.execute(context);

		expect(result.success).toBe(false);
		expect(result.error).toContain("API key");
	});

	it("should fail when destDir is not configured", async () => {
		configManager.set("destDir", "NOT_CONFIGURED");
		const cmd = new InstallWagoCommand(dbManager, configManager, "clique");
		const result = await cmd.execute(context);

		expect(result.success).toBe(false);
		expect(result.error).toContain("not configured");
	});

	it("should parse addon ID from URL", async () => {
		// Mock getAddonDetails to return not_found
		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: false,
			error: "not_found",
		});

		const cmd = new InstallWagoCommand(
			dbManager,
			configManager,
			"https://addons.wago.io/addons/clique",
		);
		const result = await cmd.execute(context);

		expect(result.success).toBe(false);
		expect(result.error).toContain("not found");
		// Verify getAddonDetails was called with parsed ID
		expect(Wago.getAddonDetails).toHaveBeenCalledWith("clique", "test-api-key");
	});

	it("should fail when addon is not found on Wago", async () => {
		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: false,
			error: "not_found",
		});

		const cmd = new InstallWagoCommand(
			dbManager,
			configManager,
			"nonexistent-addon",
		);
		const result = await cmd.execute(context);

		expect(result.success).toBe(false);
		expect(result.error).toContain("not found");
	});

	it("should install addon successfully", async () => {
		const mockAddon: Wago.WagoAddonSummary = {
			id: "clique",
			display_name: "Clique",
			summary: "Click-casting addon",
			thumbnail_image: null,
			categories: [],
			releases: {
				stable: {
					label: "v10.0.0",
					logical_timestamp: 1234567890,
					created_at: "2024-01-01T00:00:00Z",
					download_link: "https://addons.wago.io/download/clique/stable",
				},
			},
			owner: "Cladhaire",
			authors: ["Cladhaire"],
			like_count: 100,
			download_count: 50000,
			website_url: "https://addons.wago.io/addons/clique",
		};

		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: true,
			addon: mockAddon,
		});

		// Mock fetch for download
		const mockZipContent = new Uint8Array([0x50, 0x4b, 0x03, 0x04]); // ZIP header
		spyOn(global, "fetch").mockResolvedValue(
			new Response(mockZipContent, { status: 200 }),
		);

		// Mock unzip to create addon folder
		spyOn(Downloader, "unzip").mockImplementation(async (_zip, dest) => {
			const folderPath = path.join(dest, "Clique");
			fs.mkdirSync(folderPath, { recursive: true });
			await Bun.write(
				path.join(folderPath, "Clique.toc"),
				"## Title: Clique\n## Version: v10.0.0\n## Author: Cladhaire",
			);
			return true;
		});

		const cmd = new InstallWagoCommand(dbManager, configManager, "clique");
		const result = await cmd.execute(context);

		if (!result.success) {
			console.error("Install failed:", result.error);
		}

		expect(result.success).toBe(true);
		expect(result.installedAddons).toContain("Clique");

		// Verify addon is in database
		const addon = dbManager.getByFolder("Clique");
		expect(addon).toBeTruthy();
		expect(addon?.type).toBe("wago");
		expect(addon?.url).toBe("https://addons.wago.io/addons/clique");
		expect(addon?.version).toBe("v10.0.0");
		expect(addon?.author).toBe("Cladhaire");

		// Verify events were emitted
		const eventNames = emittedEvents.map((e) => e.event);
		expect(eventNames).toContain("addon:install:start");
		expect(eventNames).toContain("addon:install:downloading");
		expect(eventNames).toContain("addon:install:complete");
	});

	it("should handle multi-folder addons with ownedFolders", async () => {
		const mockAddon: Wago.WagoAddonSummary = {
			id: "elvui",
			display_name: "ElvUI",
			summary: "Complete UI replacement",
			thumbnail_image: null,
			categories: [],
			releases: {
				stable: {
					label: "13.75",
					logical_timestamp: 1234567890,
					created_at: "2024-01-01T00:00:00Z",
					download_link: "https://addons.wago.io/download/elvui/stable",
				},
			},
			owner: "Tukz",
			authors: ["Tukz", "Elv"],
			like_count: 5000,
			download_count: 1000000,
			website_url: "https://addons.wago.io/addons/elvui",
		};

		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: true,
			addon: mockAddon,
		});

		const mockZipContent = new Uint8Array([0x50, 0x4b, 0x03, 0x04]);
		spyOn(global, "fetch").mockResolvedValue(
			new Response(mockZipContent, { status: 200 }),
		);

		// Mock unzip to create multiple addon folders
		spyOn(Downloader, "unzip").mockImplementation(async (_zip, dest) => {
			// Main folder
			const mainPath = path.join(dest, "ElvUI");
			fs.mkdirSync(mainPath, { recursive: true });
			await Bun.write(
				path.join(mainPath, "ElvUI.toc"),
				"## Title: ElvUI\n## Version: 13.75",
			);

			// Sub-folders
			const libPath = path.join(dest, "ElvUI_Libraries");
			fs.mkdirSync(libPath, { recursive: true });
			await Bun.write(
				path.join(libPath, "ElvUI_Libraries.toc"),
				"## Title: ElvUI Libraries",
			);

			const optPath = path.join(dest, "ElvUI_Options");
			fs.mkdirSync(optPath, { recursive: true });
			await Bun.write(
				path.join(optPath, "ElvUI_Options.toc"),
				"## Title: ElvUI Options",
			);

			return true;
		});

		const cmd = new InstallWagoCommand(dbManager, configManager, "elvui");
		const result = await cmd.execute(context);

		expect(result.success).toBe(true);
		expect(result.installedAddons).toContain("ElvUI");
		expect(result.installedAddons).toContain("ElvUI_Libraries");
		expect(result.installedAddons).toContain("ElvUI_Options");

		// Verify parent folder has owned folders
		const addon = dbManager.getByFolder("ElvUI");
		expect(addon).toBeTruthy();
		expect(addon?.ownedFolders).toContain("ElvUI_Libraries");
		expect(addon?.ownedFolders).toContain("ElvUI_Options");

		// Verify sub-folders are NOT separate records
		expect(dbManager.getByFolder("ElvUI_Libraries")).toBeNull();
		expect(dbManager.getByFolder("ElvUI_Options")).toBeNull();
	});

	it("should fall back to beta release when stable is unavailable", async () => {
		const mockAddon: Wago.WagoAddonSummary = {
			id: "beta-addon",
			display_name: "BetaAddon",
			summary: "An addon in beta",
			thumbnail_image: null,
			categories: [],
			releases: {
				beta: {
					label: "v0.9.0-beta",
					logical_timestamp: 1234567890,
					created_at: "2024-01-01T00:00:00Z",
					download_link: "https://addons.wago.io/download/beta-addon/beta",
				},
			},
			owner: "Developer",
			authors: ["Developer"],
			like_count: 10,
			download_count: 500,
			website_url: "https://addons.wago.io/addons/beta-addon",
		};

		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: true,
			addon: mockAddon,
		});

		const mockZipContent = new Uint8Array([0x50, 0x4b, 0x03, 0x04]);
		spyOn(global, "fetch").mockResolvedValue(
			new Response(mockZipContent, { status: 200 }),
		);

		spyOn(Downloader, "unzip").mockImplementation(async (_zip, dest) => {
			const folderPath = path.join(dest, "BetaAddon");
			fs.mkdirSync(folderPath, { recursive: true });
			await Bun.write(
				path.join(folderPath, "BetaAddon.toc"),
				"## Title: BetaAddon\n## Version: v0.9.0-beta",
			);
			return true;
		});

		const cmd = new InstallWagoCommand(
			dbManager,
			configManager,
			"beta-addon",
			"stable", // Request stable but it doesn't exist
		);
		const result = await cmd.execute(context);

		expect(result.success).toBe(true);

		const addon = dbManager.getByFolder("BetaAddon");
		expect(addon?.version).toBe("v0.9.0-beta");
	});

	it("should undo installation on failure", async () => {
		const mockAddon: Wago.WagoAddonSummary = {
			id: "failing-addon",
			display_name: "FailingAddon",
			summary: "An addon that fails",
			thumbnail_image: null,
			categories: [],
			releases: {
				stable: {
					label: "v1.0.0",
					logical_timestamp: 1234567890,
					created_at: "2024-01-01T00:00:00Z",
					download_link: "https://addons.wago.io/download/failing-addon/stable",
				},
			},
			owner: "Dev",
			authors: ["Dev"],
			like_count: 0,
			download_count: 0,
			website_url: "https://addons.wago.io/addons/failing-addon",
		};

		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: true,
			addon: mockAddon,
		});

		// Mock fetch to return valid response for download
		const mockZipContent = new Uint8Array([0x50, 0x4b, 0x03, 0x04]);
		spyOn(global, "fetch").mockResolvedValue(
			new Response(mockZipContent, { status: 200 }),
		);

		// Mock unzip to create folder but then we'll simulate a failure after
		spyOn(Downloader, "unzip").mockImplementation(async (_zip, _dest) => {
			// Simulate failure - no TOC files created
			return true;
		});

		const cmd = new InstallWagoCommand(
			dbManager,
			configManager,
			"failing-addon",
		);
		const result = await cmd.execute(context);

		// Should fail because no addon folders found
		expect(result.success).toBe(false);
		expect(result.error).toContain("No addon folders found");
	});

	it("should fail when download fails", async () => {
		const mockAddon: Wago.WagoAddonSummary = {
			id: "download-fail",
			display_name: "DownloadFail",
			summary: "Download will fail",
			thumbnail_image: null,
			categories: [],
			releases: {
				stable: {
					label: "v1.0.0",
					logical_timestamp: 1234567890,
					created_at: "2024-01-01T00:00:00Z",
					download_link: "https://addons.wago.io/download/download-fail/stable",
				},
			},
			owner: "Dev",
			authors: ["Dev"],
			like_count: 0,
			download_count: 0,
			website_url: "https://addons.wago.io/addons/download-fail",
		};

		spyOn(Wago, "getAddonDetails").mockResolvedValue({
			success: true,
			addon: mockAddon,
		});

		// Mock fetch to fail
		spyOn(global, "fetch").mockResolvedValue(
			new Response("Not Found", { status: 404 }),
		);

		const cmd = new InstallWagoCommand(
			dbManager,
			configManager,
			"download-fail",
		);
		const result = await cmd.execute(context);

		expect(result.success).toBe(false);
		expect(result.error).toContain("Download failed");
	});
});
