import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ConfigManager } from "@/core/config";
import { type AddonRecord, DatabaseManager } from "@/core/db";
import * as GitClient from "@/core/git";
import { AddonManager } from "@/core/manager";

describe("Update Check Synchronization", () => {
  let tempDir: string;
  let configManager: ConfigManager;
  let dbManager: DatabaseManager;
  let addonManager: AddonManager;

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "lemonup-sync-test-"));
    const configDir = path.join(tempDir, "config");
    fs.mkdirSync(configDir, { recursive: true });

    configManager = new ConfigManager({ cwd: configDir });
    configManager.createDefaultConfig();
    configManager.set("checkInterval", 60000); // 1 minute
    dbManager = new DatabaseManager(configDir);
    addonManager = new AddonManager(configManager);
  });

  afterEach(() => {
    mock.restore();
    dbManager.close();
    fs.rmSync(tempDir, { recursive: true, force: true });
  });

  it("should perform live check when last_checked is null", async () => {
    // Add GitHub addon
    const addon: AddonRecord = {
      name: "TestAddon",
      folder: "TestAddon",
      ownedFolders: [],
      kind: "addon",
      kindOverride: false,
      flavor: "retail",
      version: "hash123",
      git_commit: "hash123",
      author: "Test",
      interface: "10000",
      url: "https://github.com/test/test",
      type: "github",
      requiredDeps: [],
      optionalDeps: [],
      embeddedLibs: [],
      install_date: new Date().toISOString(),
      last_updated: new Date().toISOString(),
      last_checked: null,
      remote_version: null,
    };
    dbManager.addAddon(addon);

    // Mock Git client
    const gitSpy = spyOn(GitClient, "getRemoteCommit").mockResolvedValue(
      "hash456",
    );

    // Check
    const result = await addonManager.checkUpdate(addon);

    expect(result.cached).toBeFalsy();
    expect(result.updateAvailable).toBe(true);
    expect(result.remoteVersion).toBe("hash456");
    expect(gitSpy).toHaveBeenCalled();

    // Verify DB update
    const updated = dbManager.getByFolder("TestAddon");
    expect(updated?.last_checked).not.toBeNull();
    expect(updated?.remote_version).toBe("hash456");
  });

  it("should return cached result when check is recent", async () => {
    const now = new Date().toISOString();
    const addon: AddonRecord = {
      name: "TestAddon",
      folder: "TestAddon",
      ownedFolders: [],
      kind: "addon",
      kindOverride: false,
      flavor: "retail",
      version: "hash123",
      git_commit: "hash123",
      author: "Test",
      interface: "10000",
      url: "https://github.com/test/test",
      type: "github",
      requiredDeps: [],
      optionalDeps: [],
      embeddedLibs: [],
      install_date: now,
      last_updated: now,
      last_checked: now, // Just checked
      remote_version: "hash456", // Known update
    };
    dbManager.addAddon(addon);

    const gitSpy = spyOn(GitClient, "getRemoteCommit").mockResolvedValue(
      "hash789",
    );

    const result = await addonManager.checkUpdate(addon);

    expect(result.cached).toBe(true);
    expect(result.updateAvailable).toBe(true);
    expect(result.remoteVersion).toBe("hash456"); // Cached version, not 789
    expect(gitSpy).not.toHaveBeenCalled();
  });

  it("should force live check when requested", async () => {
    const now = new Date().toISOString();
    const addon: AddonRecord = {
      name: "TestAddon",
      folder: "TestAddon",
      ownedFolders: [],
      kind: "addon",
      kindOverride: false,
      flavor: "retail",
      version: "hash123",
      git_commit: "hash123",
      author: "Test",
      interface: "10000",
      url: "https://github.com/test/test",
      type: "github",
      requiredDeps: [],
      optionalDeps: [],
      embeddedLibs: [],
      install_date: now,
      last_updated: now,
      last_checked: now,
      remote_version: "hash456",
    };
    dbManager.addAddon(addon);

    const gitSpy = spyOn(GitClient, "getRemoteCommit").mockResolvedValue(
      "hash789",
    );

    const result = await addonManager.checkUpdate(addon, true); // Force

    expect(result.cached).toBeFalsy();
    expect(result.remoteVersion).toBe("hash789");
    expect(gitSpy).toHaveBeenCalled();

    const updated = dbManager.getByFolder("TestAddon");
    expect(updated?.remote_version).toBe("hash789");
  });
});
