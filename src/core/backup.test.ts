import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { BackupManager } from "./backup";

const TMP_BASE = path.join(os.tmpdir(), "lemonup-tests-backup");
const RETAIL_DIR = path.join(TMP_BASE, "_retail_");
const ADDONS_DIR = path.join(RETAIL_DIR, "Interface", "AddOns");
const WTF_DIR = path.join(RETAIL_DIR, "WTF");
const BACKUPS_DIR = path.join(RETAIL_DIR, "Backups");
const BACKUPS_WTF_DIR = path.join(BACKUPS_DIR, "WTF");

describe("BackupManager", () => {
	beforeEach(async () => {
		if (fs.existsSync(TMP_BASE)) {
			fs.rmSync(TMP_BASE, { recursive: true, force: true });
		}
		fs.mkdirSync(ADDONS_DIR, { recursive: true });
		fs.mkdirSync(WTF_DIR, { recursive: true });

		await Bun.write(path.join(WTF_DIR, "Config.wtf"), "dummy content");
	});

	afterEach(() => {
		if (fs.existsSync(TMP_BASE)) {
			fs.rmSync(TMP_BASE, { recursive: true, force: true });
		}
	});

	test("should fail if WTF directory is missing", async () => {
		const missingWtfDir = path.join(TMP_BASE, "missing", "Interface", "AddOns");
		// Ensure parent exists so we don't fail on traversing
		fs.mkdirSync(path.join(TMP_BASE, "missing", "Interface"), {
			recursive: true,
		});

		await expect(BackupManager.backupWTF(missingWtfDir)).rejects.toThrow(
			"WTF folder missing",
		);
	});

	test("should create a backup zip", async () => {
		const backupPath = await BackupManager.backupWTF(ADDONS_DIR);

		expect(backupPath).not.toBeNull();
		expect(typeof backupPath).toBe("string");
		if (typeof backupPath === "string") {
			expect(await Bun.file(backupPath).exists()).toBe(true);
			expect(backupPath.endsWith(".zip")).toBe(true);
		}
	});

	test("should skip backup if performed too recently", async () => {
		// Create a "recent" backup manually
		fs.mkdirSync(BACKUPS_WTF_DIR, { recursive: true });
		const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
		const zipPath = path.join(BACKUPS_WTF_DIR, `WTF-${timestamp}.zip`);
		await Bun.write(zipPath, "dummy zip content");

		const now = new Date();
		fs.utimesSync(zipPath, now, now);

		// 2. Try to backup with interval
		const result = await BackupManager.backupWTF(ADDONS_DIR, 60); // 60 mins interval

		expect(result).toBe("skipped-recent");
	});

	test("should perform backup if previous was old enough", async () => {
		// Create an "old" backup
		fs.mkdirSync(BACKUPS_WTF_DIR, { recursive: true });
		const oldDate = new Date();
		oldDate.setMinutes(oldDate.getMinutes() - 61); // 61 mins ago

		const oldTimestamp = oldDate.toISOString().replace(/[:.]/g, "-");
		const zipPath = path.join(BACKUPS_WTF_DIR, `WTF-${oldTimestamp}.zip`);
		await Bun.write(zipPath, "dummy zip content");
		fs.utimesSync(zipPath, oldDate, oldDate);

		// Try to backup with interval
		const result = await BackupManager.backupWTF(ADDONS_DIR, 60);

		expect(result).not.toBe("skipped-recent");
		if (typeof result === "string") {
			expect(await Bun.file(result).exists()).toBe(true);
		}
	});

	test("cleanupBackups should remove old backups", async () => {
		fs.mkdirSync(BACKUPS_WTF_DIR, { recursive: true });

		// Create 5 dummy backups
		for (let i = 0; i < 5; i++) {
			const date = new Date();
			date.setMinutes(date.getMinutes() - i); // decreasing time
			const ts = date.toISOString().replace(/[:.]/g, "-") + i; // unique name
			await Bun.write(path.join(BACKUPS_WTF_DIR, `WTF-${ts}.zip`), "data");
		}

		// Keep only 2
		await BackupManager.cleanupBackups(ADDONS_DIR, 2);

		const files = fs.readdirSync(BACKUPS_WTF_DIR);
		expect(files.length).toBe(2);
	});
});
