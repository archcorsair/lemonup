import { describe, expect, it, beforeEach, afterEach } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ConfigManager } from "@/core/config";

describe("ConfigManager - Wago API Key", () => {
	let tempDir: string;
	let configManager: ConfigManager;
	let originalEnvKey: string | undefined;

	beforeEach(() => {
		originalEnvKey = process.env.WAGO_API_KEY;
		delete process.env.WAGO_API_KEY;
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "lemonup-config-test-"));
		configManager = new ConfigManager({ cwd: tempDir });
	});

	afterEach(() => {
		if (originalEnvKey !== undefined) {
			process.env.WAGO_API_KEY = originalEnvKey;
		} else {
			delete process.env.WAGO_API_KEY;
		}
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("should have wagoApiKey default to empty string", () => {
		const config = configManager.get();
		expect(config.wagoApiKey).toBe("");
	});

	it("should store and retrieve wagoApiKey", () => {
		configManager.set("wagoApiKey", "my-secret-key");
		const config = configManager.get();
		expect(config.wagoApiKey).toBe("my-secret-key");
	});

	describe("Wago API Key from environment", () => {
		it("should use WAGO_API_KEY env var when config is empty", () => {
			const originalEnv = process.env.WAGO_API_KEY;
			process.env.WAGO_API_KEY = "env-api-key";

			const manager = new ConfigManager({ cwd: tempDir });
			const config = manager.get();
			expect(config.wagoApiKey).toBe("env-api-key");

			if (originalEnv !== undefined) {
				process.env.WAGO_API_KEY = originalEnv;
			} else {
				delete process.env.WAGO_API_KEY;
			}
		});

		it("should prefer stored config over env var", () => {
			const originalEnv = process.env.WAGO_API_KEY;
			process.env.WAGO_API_KEY = "env-api-key";

			const manager = new ConfigManager({ cwd: tempDir });
			manager.set("wagoApiKey", "stored-api-key");
			const config = manager.get();
			expect(config.wagoApiKey).toBe("stored-api-key");

			if (originalEnv !== undefined) {
				process.env.WAGO_API_KEY = originalEnv;
			} else {
				delete process.env.WAGO_API_KEY;
			}
		});

		it("should use WAGO_API_KEY env var even when config fails Zod validation", () => {
			const originalEnv = process.env.WAGO_API_KEY;
			process.env.WAGO_API_KEY = "env-api-key-fallback";

			// Write config that passes conf's loose schema but fails Zod's strict validation
			// defaultMenuOption "invalid" passes conf (it's a string) but fails Zod enum
			const configPath = path.join(tempDir, "config.json");
			fs.writeFileSync(
				configPath,
				JSON.stringify({
					destDir: "/some/path",
					defaultMenuOption: "invalid_menu_option",
					wagoApiKey: "",
				}),
			);

			const manager = new ConfigManager({ cwd: tempDir });
			const config = manager.get();
			expect(config.wagoApiKey).toBe("env-api-key-fallback");

			if (originalEnv !== undefined) {
				process.env.WAGO_API_KEY = originalEnv;
			} else {
				delete process.env.WAGO_API_KEY;
			}
		});
	});
});
