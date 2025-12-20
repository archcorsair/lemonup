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
import * as GitClient from "@/core/git";
import type { AddonManager as AddonManagerType } from "@/core/manager";

// Import AddonManager
const { AddonManager } = await import("@/core/manager");

const TMP_BASE = path.join(os.tmpdir(), "lemonup-tests-short-hash");
const CONFIG_DIR = path.join(TMP_BASE, "config");
const DEST_DIR = path.join(TMP_BASE, "AddOns");

describe("AddonManager - Short Hash", () => {
	let configManager: ConfigManager;
	let manager: AddonManagerType;

	beforeEach(() => {
		// Setup Spies
		spyOn(GitClient, "getRemoteCommit").mockImplementation(() =>
			Promise.resolve("hash"),
		);

		// Setup FS
		if (fs.existsSync(TMP_BASE)) {
			fs.rmSync(TMP_BASE, { recursive: true, force: true });
		}
		fs.mkdirSync(CONFIG_DIR, { recursive: true });
		fs.mkdirSync(DEST_DIR, { recursive: true });

		// Setup Config
		configManager = new ConfigManager({ cwd: CONFIG_DIR });
		configManager.createDefaultConfig();
		configManager.set("destDir", DEST_DIR);

		manager = new AddonManager(configManager);
	});

	afterEach(() => {
		mock.restore();
		if (manager) manager.close();
		if (fs.existsSync(TMP_BASE)) {
			try {
				fs.rmSync(TMP_BASE, { recursive: true, force: true });
			} catch (_) {}
		}
	});

	test("checkUpdate should return false if local short hash matches remote full hash prefix", async () => {
		const fullHash = "0a21ee9393304d1303866162a04297127e2a9792";
		const shortHash = "0a21ee9";

		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "short-hash-repo",
			type: "github",
			url: "http://git",
			version: "v1.0",
			git_commit: shortHash,
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue(fullHash);

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(false);
		expect(result.remoteVersion).toBe(fullHash);
	});

	test("checkUpdate should return false if local full hash matches remote short hash prefix (rare but possible)", async () => {
		const fullHash = "0a21ee9393304d1303866162a04297127e2a9792";
		const shortHash = "0a21ee9";

		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "full-hash-repo",
			type: "github",
			url: "http://git",
			version: "v1.0",
			git_commit: fullHash,
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue(shortHash);

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(false);
	});

	test("checkUpdate should return true if hashes are completely different", async () => {
		const fullHash = "0a21ee9393304d1303866162a04297127e2a9792";
		const otherHash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

		// biome-ignore lint/suspicious/noExplicitAny: test data
		const addon: any = {
			name: "diff-hash-repo",
			type: "github",
			url: "http://git",
			version: "v1.0",
			git_commit: otherHash,
		};

		spyOn(GitClient, "getRemoteCommit").mockResolvedValue(fullHash);

		const result = await manager.checkUpdate(addon);

		expect(result.updateAvailable).toBe(true);
	});
});
