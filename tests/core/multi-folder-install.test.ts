import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import fs from "node:fs/promises";
import path from "node:path";
import { DatabaseManager } from "@/core/db";
import { ConfigManager } from "@/core/config";
import { ScanCommand } from "@/core/commands/ScanCommand";

describe("Multi-folder addon installation", () => {
  const tempDir = path.join(process.cwd(), "test-output", "MultiFolderInstall");
  const addonsDir = path.join(tempDir, "Interface", "AddOns");
  const configDir = path.join(tempDir, "Config");

  let dbManager: DatabaseManager;
  let configManager: ConfigManager;

  beforeEach(async () => {
    try {
      await fs.rm(tempDir, { recursive: true, force: true });
    } catch (err: any) {
      if (err?.code !== "EBUSY") {
        throw err;
      }
    }
    await fs.mkdir(addonsDir, { recursive: true });
    await fs.mkdir(configDir, { recursive: true });

    configManager = new ConfigManager({ cwd: configDir });
    configManager.set("destDir", addonsDir);
    dbManager = new DatabaseManager(configDir);
  });

  afterEach(async () => {
    dbManager.close();
    try {
      await fs.rm(tempDir, { recursive: true, force: true });
    } catch (err: any) {
      if (err?.code !== "EBUSY") {
        throw err;
      }
    }
  });

  it("should treat all folders from a zip as owned by the parent addon", async () => {
    // Simulate a multi-folder addon like Details!
    // Create folders: Details, Details_DataStorage, Details_EncounterDetails
    const detailsDir = path.join(addonsDir, "Details");
    const dataStorageDir = path.join(addonsDir, "Details_DataStorage");
    const encounterDir = path.join(addonsDir, "Details_EncounterDetails");

    await fs.mkdir(detailsDir, { recursive: true });
    await fs.mkdir(dataStorageDir, { recursive: true });
    await fs.mkdir(encounterDir, { recursive: true });

    // Create minimal TOC files
    await fs.writeFile(
      path.join(detailsDir, "Details.toc"),
      `## Interface: 110002
## Title: Details
## Version: 1.0.0
## Author: Terciansen`
    );
    await fs.writeFile(
      path.join(dataStorageDir, "Details_DataStorage.toc"),
      `## Interface: 110002
## Title: Details DataStorage
## Version: 1.0.0`
    );
    await fs.writeFile(
      path.join(encounterDir, "Details_EncounterDetails.toc"),
      `## Interface: 110002
## Title: Details EncounterDetails
## Version: 1.0.0`
    );

    // Register the main addon with owned folders (simulating post-install state)
    const allFolders = [
      "Details",
      "Details_DataStorage",
      "Details_EncounterDetails",
    ];
    const parentFolder = "Details";
    const ownedFolders = allFolders.filter((f) => f !== parentFolder);

    // This is what InstallFromUrlCommand should do after install
    dbManager.addAddon({
      name: "Details",
      folder: parentFolder,
      ownedFolders,
      kind: "addon",
      kindOverride: false,
      flavor: "retail",
      version: "1.0.0",
      git_commit: null,
      author: "Terciansen",
      interface: "110002",
      url: "https://github.com/Terciansen/Details",
      type: "github",
      requiredDeps: [],
      optionalDeps: [],
      embeddedLibs: [],
      install_date: new Date().toISOString(),
      last_updated: new Date().toISOString(),
    });

    // Verify: parent addon should have owned folders
    const parent = dbManager.getByFolder(parentFolder);
    expect(parent).not.toBeNull();
    expect(parent!.ownedFolders).toEqual(ownedFolders);

    // Verify: owned folders should NOT have their own DB records
    const dataStorage = dbManager.getByFolder("Details_DataStorage");
    const encounter = dbManager.getByFolder("Details_EncounterDetails");
    expect(dataStorage).toBeNull();
    expect(encounter).toBeNull();

    // Verify: getOwnerOf should return the parent
    const ownerOfDataStorage = dbManager.getOwnerOf("Details_DataStorage");
    expect(ownerOfDataStorage).not.toBeNull();
    expect(ownerOfDataStorage!.folder).toBe(parentFolder);
  });

  it("should scan only parent folder and set ownedFolders for others", async () => {
    // Create a mock multi-folder structure as if extracted from zip
    const detailsDir = path.join(addonsDir, "Details");
    const dataStorageDir = path.join(addonsDir, "Details_DataStorage");

    await fs.mkdir(detailsDir, { recursive: true });
    await fs.mkdir(dataStorageDir, { recursive: true });

    await fs.writeFile(
      path.join(detailsDir, "Details.toc"),
      `## Interface: 110002
## Title: Details
## Version: 1.0.0
## Author: Terciansen`
    );
    await fs.writeFile(
      path.join(dataStorageDir, "Details_DataStorage.toc"),
      `## Interface: 110002
## Title: Details DataStorage
## Version: 1.0.0`
    );

    // Simulate what InstallFromUrlCommand should do:
    // 1. Detect all folders: ["Details", "Details_DataStorage"]
    // 2. Determine parent: "Details"
    // 3. Scan ONLY parent
    // 4. Update parent with ownedFolders

    const allFolders = ["Details", "Details_DataStorage"];
    const parentFolder = "Details";
    const ownedFolders = allFolders.filter((f) => f !== parentFolder);

    // Scan only parent
    const scanCmd = new ScanCommand(dbManager, configManager, [parentFolder]);
    await scanCmd.execute({
      emit: () => {},
      getConfig: () => ({
        wowPath: tempDir,
        flavor: "retail",
        destDir: addonsDir,
      }),
    } as any);

    // Update parent with ownership
    dbManager.updateAddon(parentFolder, {
      ownedFolders,
    });

    // Verify
    const parent = dbManager.getByFolder(parentFolder);
    expect(parent).not.toBeNull();
    expect(parent!.ownedFolders).toEqual(["Details_DataStorage"]);

    const child = dbManager.getByFolder("Details_DataStorage");
    expect(child).toBeNull();
  });

  it("should handle single-folder addon without ownedFolders", async () => {
    const addonDir = path.join(addonsDir, "SimpleAddon");
    await fs.mkdir(addonDir, { recursive: true });

    await fs.writeFile(
      path.join(addonDir, "SimpleAddon.toc"),
      `## Interface: 110002
## Title: Simple Addon
## Version: 1.0.0`
    );

    // Scan the single addon
    const scanCmd = new ScanCommand(dbManager, configManager, ["SimpleAddon"]);
    await scanCmd.execute({
      emit: () => {},
      getConfig: () => ({
        wowPath: tempDir,
        flavor: "retail",
        destDir: addonsDir,
      }),
    } as any);

    // Verify: single-folder addon should have empty ownedFolders
    const addon = dbManager.getByFolder("SimpleAddon");
    expect(addon).not.toBeNull();
    expect(addon!.ownedFolders).toEqual([]);
  });
});
