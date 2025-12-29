import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ConfigManager } from "@/core/config";

const TMP_DIR = path.join(os.tmpdir(), "lemonup-tests-config");

describe("ConfigManager", () => {
	beforeEach(() => {
		if (fs.existsSync(TMP_DIR)) {
			fs.rmSync(TMP_DIR, { recursive: true, force: true });
		}
		fs.mkdirSync(TMP_DIR, { recursive: true });
	});

	afterEach(() => {
		if (fs.existsSync(TMP_DIR)) {
			fs.rmSync(TMP_DIR, { recursive: true, force: true });
		}
	});

	test("should initialize with default values if config is missing", () => {
		const manager = new ConfigManager({ cwd: TMP_DIR });
		const config = manager.get();

		expect(config.destDir).toBe("NOT_CONFIGURED");
		expect(config.repositories).toEqual([]);
	});

	test("should create default config", () => {
		const fs = require("node:fs");
		const originalExistsSync = fs.existsSync;
		// biome-ignore lint/suspicious/noExplicitAny: path can be string or URL
		const spy = spyOn(fs, "existsSync").mockImplementation((p: any) => {
			if (typeof p === "string" && p.includes(".wine")) return true;
			// Default behavior for other paths (needed for ConfigManager internals)
			if (p === TMP_DIR || (typeof p === "string" && p.startsWith(TMP_DIR)))
				return originalExistsSync(p);
			return false;
		});
		// Hack: Force os.platform to linux if not already, or ensure logical path
		spyOn(os, "platform").mockReturnValue("linux");

		const manager = new ConfigManager({ cwd: TMP_DIR });
		manager.createDefaultConfig();
		const config = manager.get();

		// destDir defaults to NOT_CONFIGURED - wizard or user will set it later
		expect(config.destDir).toBe("NOT_CONFIGURED");
		expect(config.checkInterval).toBe(60000 * 5);
		expect(manager.hasConfigFile).toBe(true);

		spy.mockRestore();
	});

	test("should set and get values", () => {
		const manager = new ConfigManager({ cwd: TMP_DIR });
		manager.createDefaultConfig();

		manager.set("destDir", "/tmp/wow");
		manager.set("debug", true);

		const config = manager.get();
		expect(config.destDir).toBe("/tmp/wow");
		expect(config.debug).toBe(true);
	});

	test("should respect overrides", () => {
		const manager = new ConfigManager({
			cwd: TMP_DIR,
			overrides: { debug: true },
		});

		const config = manager.get();
		expect(config.debug).toBe(true);
	});

	test("should not save in safe mode", () => {
		const manager = new ConfigManager({
			cwd: TMP_DIR,
			enableSafeMode: true,
		});
		manager.createDefaultConfig();

		manager.set("destDir", "/tmp/safe");

		const manager2 = new ConfigManager({ cwd: TMP_DIR });
		const config2 = manager2.get();
		expect(config2.destDir).not.toBe("/tmp/safe");
	});

	test("should default theme to dark and allow updates", () => {
		const manager = new ConfigManager({ cwd: TMP_DIR });
		manager.createDefaultConfig();

		// Default
		expect(manager.get().theme).toBe("dark");

		// Update
		manager.set("theme", "light");
		expect(manager.get().theme).toBe("light");

		// Invalid should throw or be ignored (depending on Zod implementation details, conf might just throw)
		// We'll trust Zod validation here but verify the success path primarily.
	});
});
