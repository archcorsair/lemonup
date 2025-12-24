import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import fs from "node:fs/promises";
import path from "node:path";
import { ScanCommand } from "@/core/commands/ScanCommand";
import { ConfigManager } from "@/core/config";
import { DatabaseManager } from "@/core/db";
import type { CommandContext } from "@/core/commands/types";

describe("ScanCommand Relationships", () => {
	const tempDir = path.join(process.cwd(), "test-output", "ScanRelationships");
	const addonsDir = path.join(tempDir, "Interface", "AddOns");
	const configDir = path.join(tempDir, "Config");
	
	let configManager: ConfigManager;
	let dbManager: DatabaseManager;

	beforeEach(async () => {
		await fs.rm(tempDir, { recursive: true, force: true });
		await fs.mkdir(addonsDir, { recursive: true });
		await fs.mkdir(configDir, { recursive: true });

		configManager = new ConfigManager({ cwd: configDir });
		configManager.set("destDir", addonsDir);
		dbManager = new DatabaseManager(configDir);
	});

	afterEach(async () => {
		dbManager.close();
		// await fs.rm(tempDir, { recursive: true, force: true });
	});

	test("should link child addon based on prefix", async () => {
		// Create Parent
		await fs.mkdir(path.join(addonsDir, "ElvUI"), { recursive: true });
		await fs.writeFile(path.join(addonsDir, "ElvUI", "ElvUI.toc"), "## Title: ElvUI\n## Version: 1.0");

		// Create Child
		await fs.mkdir(path.join(addonsDir, "ElvUI_OptionsUI"), { recursive: true });
		await fs.writeFile(path.join(addonsDir, "ElvUI_OptionsUI", "ElvUI_OptionsUI.toc"), "## Title: ElvUI Options\n## Version: 1.0");

		const cmd = new ScanCommand(dbManager, configManager);
		const context: CommandContext = { emit: () => {} };
		await cmd.execute(context);

		const elvui = dbManager.getByFolder("ElvUI");
		const options = dbManager.getByFolder("ElvUI_OptionsUI");

		expect(elvui).not.toBeNull();
		expect(options).not.toBeNull();
		expect(elvui?.parent).toBeNull();
		expect(options?.parent).toBe("ElvUI");
	});

	test("should link child addon based on dependency and name similarity", async () => {
		// Create Core
		await fs.mkdir(path.join(addonsDir, "DBM-Core"), { recursive: true });
		await fs.writeFile(path.join(addonsDir, "DBM-Core", "DBM-Core.toc"), "## Title: DBM Core\n## Version: 1.0");

		// Create Module
		await fs.mkdir(path.join(addonsDir, "DBM-Naxx"), { recursive: true });
		await fs.writeFile(path.join(addonsDir, "DBM-Naxx", "DBM-Naxx.toc"), "## Title: DBM Naxx\n## Dependencies: DBM-Core\n## Version: 1.0");

		const cmd = new ScanCommand(dbManager, configManager);
		const context: CommandContext = { emit: () => {} };
		await cmd.execute(context);

		const core = dbManager.getByFolder("DBM-Core");
		const naxx = dbManager.getByFolder("DBM-Naxx");

		expect(core).not.toBeNull();
		expect(naxx).not.toBeNull();
		expect(core?.parent).toBeNull();
		expect(naxx?.parent).toBe("DBM-Core");
	});
	
	test("should NOT link unrelated addons despite dependency", async () => {
		// Create Lib
		await fs.mkdir(path.join(addonsDir, "Ace3"), { recursive: true });
		await fs.writeFile(path.join(addonsDir, "Ace3", "Ace3.toc"), "## Title: Ace3\n## Version: 1.0");

		// Create Consumer
		await fs.mkdir(path.join(addonsDir, "WeakAuras"), { recursive: true });
		await fs.writeFile(path.join(addonsDir, "WeakAuras", "WeakAuras.toc"), "## Title: WeakAuras\n## Dependencies: Ace3\n## Version: 1.0");

		const cmd = new ScanCommand(dbManager, configManager);
		const context: CommandContext = { emit: () => {} };
		await cmd.execute(context);

		const lib = dbManager.getByFolder("Ace3");
		const app = dbManager.getByFolder("WeakAuras");

		expect(lib).not.toBeNull();
		expect(app).not.toBeNull();
		expect(lib?.parent).toBeNull();
		expect(app?.parent).toBeNull(); // Should NOT be linked to Ace3
	});
});
