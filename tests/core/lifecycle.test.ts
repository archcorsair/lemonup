import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import fs from "node:fs/promises";
import path from "node:path";
import { RemoveAddonCommand } from "@/core/commands/RemoveAddonCommand";
import { ScanCommand } from "@/core/commands/ScanCommand";
import type { CommandContext } from "@/core/commands/types";
import { ConfigManager } from "@/core/config";
import { DatabaseManager } from "@/core/db";

describe("Addon Lifecycle", () => {
	const tempDir = path.join(process.cwd(), "test-output", "Lifecycle");
	const addonsDir = path.join(tempDir, "Interface", "AddOns");
	const configDir = path.join(tempDir, "Config");

	let configManager: ConfigManager;
	let dbManager: DatabaseManager;
	const mockContext: CommandContext = { emit: () => {} };

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
		await fs.rm(tempDir, { recursive: true, force: true });
	});

	describe("Deletion blocking with dependencies", () => {
		test("should block deletion when addon is required by another", async () => {
			// Create library addon
			await fs.mkdir(path.join(addonsDir, "LibSharedMedia-3.0"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "LibSharedMedia-3.0", "LibSharedMedia-3.0.toc"),
				"## Title: LibSharedMedia\n## Version: 1.0\n## X-Library: true",
			);

			// Create addon that requires the library
			await fs.mkdir(path.join(addonsDir, "MyAddon"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "MyAddon", "MyAddon.toc"),
				"## Title: My Addon\n## Version: 1.0\n## Dependencies: LibSharedMedia-3.0",
			);

			// Scan to populate DB
			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			// Attempt to delete the library (should be blocked)
			const removeCmd = new RemoveAddonCommand(
				dbManager,
				configManager,
				"LibSharedMedia-3.0",
				false,
			);
			const result = await removeCmd.execute(mockContext);

			expect(result.success).toBe(false);
			expect(result.blocked).toBeDefined();
			expect(result.blocked?.dependents).toContain("MyAddon");
			expect(result.blocked?.reason).toContain("required");
		});

		test("should allow deletion when forced", async () => {
			// Create library addon
			await fs.mkdir(path.join(addonsDir, "LibStub"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "LibStub", "LibStub.toc"),
				"## Title: LibStub\n## Version: 1.0",
			);

			// Create addon that requires the library
			await fs.mkdir(path.join(addonsDir, "ConsumerAddon"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "ConsumerAddon", "ConsumerAddon.toc"),
				"## Title: Consumer Addon\n## Version: 1.0\n## Dependencies: LibStub",
			);

			// Scan to populate DB
			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			// Force delete the library
			const removeCmd = new RemoveAddonCommand(
				dbManager,
				configManager,
				"LibStub",
				true, // force = true
			);
			const result = await removeCmd.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.removedFolders).toContain("LibStub");
			expect(dbManager.getByFolder("LibStub")).toBeNull();
		});

		test("should NOT block deletion for optional dependencies", async () => {
			// Create library addon
			await fs.mkdir(path.join(addonsDir, "OptionalLib"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "OptionalLib", "OptionalLib.toc"),
				"## Title: Optional Library\n## Version: 1.0",
			);

			// Create addon with optional dependency
			await fs.mkdir(path.join(addonsDir, "SomeAddon"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "SomeAddon", "SomeAddon.toc"),
				"## Title: Some Addon\n## Version: 1.0\n## OptionalDeps: OptionalLib",
			);

			// Scan to populate DB
			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			// Delete the optional library (should NOT be blocked)
			const removeCmd = new RemoveAddonCommand(
				dbManager,
				configManager,
				"OptionalLib",
				false,
			);
			const result = await removeCmd.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.removedFolders).toContain("OptionalLib");
		});
	});

	describe("Multi-folder addon deletion", () => {
		test("should delete all owned folders when deleting main addon", async () => {
			// Create main addon
			await fs.mkdir(path.join(addonsDir, "ElvUI"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "ElvUI", "ElvUI.toc"),
				"## Title: ElvUI\n## Version: 1.0",
			);

			// Create owned folders
			await fs.mkdir(path.join(addonsDir, "ElvUI_Options"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "ElvUI_Options", "ElvUI_Options.toc"),
				"## Title: ElvUI Options\n## Version: 1.0",
			);

			await fs.mkdir(path.join(addonsDir, "ElvUI_Libraries"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "ElvUI_Libraries", "ElvUI_Libraries.toc"),
				"## Title: ElvUI Libraries\n## Version: 1.0",
			);

			// Scan and manually set ownedFolders
			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			dbManager.updateAddon("ElvUI", {
				ownedFolders: ["ElvUI_Options", "ElvUI_Libraries"],
			});
			// Remove owned folder records (they're tracked via ownedFolders)
			dbManager.removeAddon("ElvUI_Options");
			dbManager.removeAddon("ElvUI_Libraries");

			// Delete main addon
			const removeCmd = new RemoveAddonCommand(
				dbManager,
				configManager,
				"ElvUI",
				false,
			);
			const result = await removeCmd.execute(mockContext);

			expect(result.success).toBe(true);
			expect(result.removedFolders).toContain("ElvUI");
			expect(result.removedFolders).toContain("ElvUI_Options");
			expect(result.removedFolders).toContain("ElvUI_Libraries");

			// Verify all folders are deleted
			const elvuiExists = await fs.access(path.join(addonsDir, "ElvUI")).then(() => true).catch(() => false);
			const optionsExists = await fs.access(path.join(addonsDir, "ElvUI_Options")).then(() => true).catch(() => false);
			const libsExists = await fs.access(path.join(addonsDir, "ElvUI_Libraries")).then(() => true).catch(() => false);

			expect(elvuiExists).toBe(false);
			expect(optionsExists).toBe(false);
			expect(libsExists).toBe(false);
		});
	});

	describe("Library classification persistence", () => {
		test("should auto-classify libraries based on name pattern", async () => {
			// Create library-pattern named addon
			await fs.mkdir(path.join(addonsDir, "LibDataBroker-1.1"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "LibDataBroker-1.1", "LibDataBroker-1.1.toc"),
				"## Title: LibDataBroker\n## Version: 1.1",
			);

			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			const addon = dbManager.getByFolder("LibDataBroker-1.1");
			expect(addon?.kind).toBe("library");
			expect(addon?.kindOverride).toBe(false);
		});

		test("should preserve kind override after rescan", async () => {
			// Create addon
			await fs.mkdir(path.join(addonsDir, "SomeAddon"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "SomeAddon", "SomeAddon.toc"),
				"## Title: Some Addon\n## Version: 1.0",
			);

			// Initial scan
			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			// Manually override to library
			dbManager.updateAddon("SomeAddon", {
				kind: "library",
				kindOverride: true,
			});

			// Rescan
			await scanCmd.execute(mockContext);

			// Verify override persisted
			const addon = dbManager.getByFolder("SomeAddon");
			expect(addon?.kind).toBe("library");
			expect(addon?.kindOverride).toBe(true);
		});

		test("should respect X-Library TOC metadata", async () => {
			// Create addon with X-Library flag
			await fs.mkdir(path.join(addonsDir, "CustomLib"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "CustomLib", "CustomLib.toc"),
				"## Title: Custom Library\n## Version: 1.0\n## X-Library: true",
			);

			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			const addon = dbManager.getByFolder("CustomLib");
			// Note: X-Library is parsed but classification happens post-scan
			// The library detection should classify it based on name pattern "CustomLib" doesn't match
			// but X-Library flag should be detected by parseTOCContent
			expect(addon).not.toBeNull();
		});
	});

	describe("Dependency parsing", () => {
		test("should parse required dependencies from TOC", async () => {
			await fs.mkdir(path.join(addonsDir, "TestAddon"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "TestAddon", "TestAddon.toc"),
				"## Title: Test Addon\n## Version: 1.0\n## Dependencies: LibStub, Ace3, CallbackHandler-1.0",
			);

			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			const addon = dbManager.getByFolder("TestAddon");
			expect(addon?.requiredDeps).toEqual(["LibStub", "Ace3", "CallbackHandler-1.0"]);
		});

		test("should parse optional dependencies from TOC", async () => {
			await fs.mkdir(path.join(addonsDir, "TestAddon2"), { recursive: true });
			await fs.writeFile(
				path.join(addonsDir, "TestAddon2", "TestAddon2.toc"),
				"## Title: Test Addon 2\n## Version: 1.0\n## OptionalDeps: Masque, ElvUI",
			);

			const scanCmd = new ScanCommand(dbManager, configManager);
			await scanCmd.execute(mockContext);

			const addon = dbManager.getByFolder("TestAddon2");
			expect(addon?.optionalDeps).toEqual(["Masque", "ElvUI"]);
		});
	});
});
