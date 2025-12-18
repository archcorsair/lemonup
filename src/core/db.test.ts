import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { type AddonRecord, DatabaseManager } from "./db";

const TMP_DIR = path.join(os.tmpdir(), "lemonup-tests-db");

describe("DatabaseManager", () => {
	let dbManager: DatabaseManager;

	beforeEach(() => {
		fs.mkdirSync(TMP_DIR, { recursive: true });
		dbManager = new DatabaseManager(TMP_DIR);
	});

	afterEach(() => {
		dbManager.close();
		if (fs.existsSync(TMP_DIR)) {
			fs.rmSync(TMP_DIR, { recursive: true, force: true });
		}
	});

	test("should create database and table", () => {
		expect(fs.existsSync(path.join(TMP_DIR, "lemonup.db"))).toBe(true);
		const addons = dbManager.getAll();
		expect(addons).toEqual([]);
	});

	test("should add and get addon", () => {
		const newAddon: AddonRecord = {
			name: "TestAddon",
			folder: "TestAddon",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "100000",
			url: "https://github.com/test/test",
			type: "github",
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(newAddon);

		const result = dbManager.getByFolder("TestAddon");
		expect(result).not.toBeNull();
		expect(result?.name).toBe("TestAddon");
		expect(result?.version).toBe("1.0.0");
	});

	test("should update addon", () => {
		const newAddon: AddonRecord = {
			name: "Test Addon",
			folder: "TestAddon",
			version: "1.0.0",
			git_commit: null,
			author: "Tester",
			interface: "100000",
			url: null,
			type: "manual",
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};
		dbManager.addAddon(newAddon);

		dbManager.updateAddon("TestAddon", {
			version: "1.0.1",
			author: "NewAuthor",
		});

		const result = dbManager.getByFolder("TestAddon");
		expect(result?.version).toBe("1.0.1");
		expect(result?.author).toBe("NewAuthor");
	});

	test("should remove addon", () => {
		const newAddon: AddonRecord = {
			name: "Test Addon",
			folder: "TestAddon",
			version: "1.0.0",
			git_commit: null,
			author: "Tester",
			interface: "100000",
			url: null,
			type: "manual",
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};
		dbManager.addAddon(newAddon);

		dbManager.removeAddon("TestAddon");

		const result = dbManager.getByFolder("TestAddon");
		expect(result).toBeNull();
	});

	test("should prevent duplicate folders", () => {
		const addon: AddonRecord = {
			name: "Test Addon",
			folder: "UniqueFolder",
			version: "1.0",
			git_commit: null,
			author: "Me",
			interface: "123",
			url: null,
			type: "manual",
			install_date: "",
			last_updated: "",
		};

		dbManager.addAddon(addon);

		expect(() => {
			dbManager.addAddon(addon);
		}).toThrow();
	});
});
