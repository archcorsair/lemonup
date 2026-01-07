import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { InstallWagoCommand } from "@/core/commands/InstallWagoCommand";
import { UpdateAddonCommand } from "@/core/commands/UpdateAddonCommand";
import type { CommandContext } from "@/core/commands/types";
import { ConfigManager } from "@/core/config";
import { DatabaseManager } from "@/core/db";

describe("Wago Integration", () => {
	// Note: These tests require WAGO_API_KEY env var or will be skipped
	const apiKey = process.env.WAGO_API_KEY;

	let tempDir: string;
	let configDir: string;
	let destDir: string;
	let configManager: ConfigManager;
	let dbManager: DatabaseManager;
	let context: CommandContext;
	let emittedEvents: Array<{ event: string; args: unknown[] }>;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(
			path.join(os.tmpdir(), "lemonup-wago-integration-"),
		);
		configDir = path.join(tempDir, "config");
		destDir = path.join(tempDir, "addons");
		fs.mkdirSync(configDir, { recursive: true });
		fs.mkdirSync(destDir, { recursive: true });

		configManager = new ConfigManager({ cwd: configDir });
		configManager.createDefaultConfig();
		configManager.set("destDir", destDir);
		if (apiKey) {
			configManager.set("wagoApiKey", apiKey);
		}
		dbManager = new DatabaseManager(configDir);

		emittedEvents = [];
		context = {
			emit: (event: string, ...args: unknown[]) => {
				emittedEvents.push({ event, args });
			},
		} as CommandContext;
	});

	afterEach(() => {
		dbManager.close();
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it.skipIf(!apiKey)("should install a real addon from Wago", async () => {
		// Use a small, real addon for testing
		const cmd = new InstallWagoCommand(dbManager, configManager, "clique");
		const result = await cmd.execute(context);

		expect(result.success).toBe(true);
		expect(result.installedAddons.length).toBeGreaterThan(0);

		// Verify addon was written to destDir
		const installedFolder = result.installedAddons[0];
		if (installedFolder) {
			expect(fs.existsSync(path.join(destDir, installedFolder))).toBe(true);

			// Verify database record
			const record = dbManager.getByFolder(installedFolder);
			expect(record).not.toBeNull();
			expect(record?.type).toBe("wago");
		}
	});

	it.skipIf(!apiKey)(
		"should update a Wago addon when new version available",
		async () => {
			// First install the addon
			const installCmd = new InstallWagoCommand(
				dbManager,
				configManager,
				"clique",
			);
			const installResult = await installCmd.execute(context);
			expect(installResult.success).toBe(true);

			const installedFolder = installResult.installedAddons[0];
			expect(installedFolder).toBeDefined();
			if (!installedFolder) return;

			// Get the current record and simulate an old version
			const record = dbManager.getByFolder(installedFolder);
			expect(record).not.toBeNull();
			if (!record) return;

			const currentVersion = record.version;
			dbManager.updateAddon(installedFolder, { version: "0.0.0-old" });

			// Verify the version was changed
			const oldRecord = dbManager.getByFolder(installedFolder);
			expect(oldRecord?.version).toBe("0.0.0-old");

			// Now run update
			emittedEvents = [];
			const updateCmd = new UpdateAddonCommand(
				dbManager,
				configManager,
				oldRecord!,
			);
			const updateResult = await updateCmd.execute(context);

			expect(updateResult.success).toBe(true);
			expect(updateResult.updated).toBe(true);

			// Verify version was updated back
			const updatedRecord = dbManager.getByFolder(installedFolder);
			expect(updatedRecord?.version).toBe(currentVersion);

			// Verify addon files still exist
			expect(fs.existsSync(path.join(destDir, installedFolder))).toBe(true);
		},
	);

	it.skipIf(!apiKey)(
		"should detect no update needed when version matches",
		async () => {
			// First install the addon
			const installCmd = new InstallWagoCommand(
				dbManager,
				configManager,
				"clique",
			);
			const installResult = await installCmd.execute(context);
			expect(installResult.success).toBe(true);

			const installedFolder = installResult.installedAddons[0];
			expect(installedFolder).toBeDefined();
			if (!installedFolder) return;

			const record = dbManager.getByFolder(installedFolder);
			expect(record).not.toBeNull();
			if (!record) return;

			// Run update without changing version - should report no update
			emittedEvents = [];
			const updateCmd = new UpdateAddonCommand(
				dbManager,
				configManager,
				record,
			);
			const updateResult = await updateCmd.execute(context);

			expect(updateResult.success).toBe(true);
			expect(updateResult.updated).toBe(false);
		},
	);
});
