import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { InstallWagoCommand } from "@/core/commands/InstallWagoCommand";
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
});
