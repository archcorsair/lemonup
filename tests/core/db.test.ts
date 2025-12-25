import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { type AddonRecord, DatabaseManager } from "@/core/db";

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
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "100000",
			url: "https://github.com/test/test",
			type: "github",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(newAddon);

		const result = dbManager.getByFolder("TestAddon");
		expect(result).not.toBeNull();
		expect(result?.name).toBe("TestAddon");
		expect(result?.version).toBe("1.0.0");
		expect(result?.ownedFolders).toEqual([]);
		expect(result?.kind).toBe("addon");
		expect(result?.kindOverride).toBe(false);
		expect(result?.flavor).toBe("retail");
		expect(result?.requiredDeps).toEqual([]);
		expect(result?.optionalDeps).toEqual([]);
		expect(result?.embeddedLibs).toEqual([]);
	});

	test("should update addon", () => {
		const newAddon: AddonRecord = {
			name: "Test Addon",
			folder: "TestAddon",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Tester",
			interface: "100000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
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
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Tester",
			interface: "100000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
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
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0",
			git_commit: null,
			author: "Me",
			interface: "123",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: "",
			last_updated: "",
		};

		dbManager.addAddon(addon);

		expect(() => {
			dbManager.addAddon(addon);
		}).toThrow();
	});

	test("should store and retrieve ownedFolders", () => {
		const addon: AddonRecord = {
			name: "ElvUI",
			folder: "ElvUI",
			ownedFolders: ["ElvUI_Options", "ElvUI_Libraries"],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(addon);

		const result = dbManager.getByFolder("ElvUI");
		expect(result?.ownedFolders).toEqual(["ElvUI_Options", "ElvUI_Libraries"]);
	});

	test("should store and retrieve dependency arrays", () => {
		const addon: AddonRecord = {
			name: "WeakAuras",
			folder: "WeakAuras",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: ["Ace3", "LibStub"],
			optionalDeps: ["Masque"],
			embeddedLibs: ["LibCompress", "LibSerialize"],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(addon);

		const result = dbManager.getByFolder("WeakAuras");
		expect(result?.requiredDeps).toEqual(["Ace3", "LibStub"]);
		expect(result?.optionalDeps).toEqual(["Masque"]);
		expect(result?.embeddedLibs).toEqual(["LibCompress", "LibSerialize"]);
	});

	test("should update array fields", () => {
		const addon: AddonRecord = {
			name: "TestAddon",
			folder: "TestAddon",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(addon);

		dbManager.updateAddon("TestAddon", {
			ownedFolders: ["TestAddon_Core"],
			requiredDeps: ["Ace3"],
			kind: "library",
			kindOverride: true,
		});

		const result = dbManager.getByFolder("TestAddon");
		expect(result?.ownedFolders).toEqual(["TestAddon_Core"]);
		expect(result?.requiredDeps).toEqual(["Ace3"]);
		expect(result?.kind).toBe("library");
		expect(result?.kindOverride).toBe(true);
	});

	test("getDependents should return addons with folder in deps", () => {
		const ace3: AddonRecord = {
			name: "Ace3",
			folder: "Ace3",
			ownedFolders: [],
			kind: "library",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		const weakAuras: AddonRecord = {
			name: "WeakAuras",
			folder: "WeakAuras",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: ["Ace3"],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		const plater: AddonRecord = {
			name: "Plater",
			folder: "Plater",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: ["Ace3"],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(ace3);
		dbManager.addAddon(weakAuras);
		dbManager.addAddon(plater);

		const dependents = dbManager.getDependents("Ace3");
		expect(dependents).toHaveLength(2);
		expect(dependents.map((a) => a.folder).sort()).toEqual(["Plater", "WeakAuras"]);
	});

	test("getRequiredDependents should return only required deps", () => {
		const ace3: AddonRecord = {
			name: "Ace3",
			folder: "Ace3",
			ownedFolders: [],
			kind: "library",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		const weakAuras: AddonRecord = {
			name: "WeakAuras",
			folder: "WeakAuras",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: ["Ace3"],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		const plater: AddonRecord = {
			name: "Plater",
			folder: "Plater",
			ownedFolders: [],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: ["Ace3"],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(ace3);
		dbManager.addAddon(weakAuras);
		dbManager.addAddon(plater);

		const requiredDependents = dbManager.getRequiredDependents("Ace3");
		expect(requiredDependents).toHaveLength(1);
		expect(requiredDependents[0]?.folder).toBe("WeakAuras");
	});

	test("getOwnerOf should return addon that owns folder", () => {
		const elvui: AddonRecord = {
			name: "ElvUI",
			folder: "ElvUI",
			ownedFolders: ["ElvUI_Options", "ElvUI_Libraries"],
			kind: "addon",
			kindOverride: false,
			flavor: "retail",
			version: "1.0.0",
			git_commit: null,
			author: "Author",
			interface: "110000",
			url: null,
			type: "manual",
			requiredDeps: [],
			optionalDeps: [],
			embeddedLibs: [],
			install_date: new Date().toISOString(),
			last_updated: new Date().toISOString(),
		};

		dbManager.addAddon(elvui);

		const ownerOfOptions = dbManager.getOwnerOf("ElvUI_Options");
		expect(ownerOfOptions).not.toBeNull();
		expect(ownerOfOptions?.folder).toBe("ElvUI");

		const ownerOfElvUI = dbManager.getOwnerOf("ElvUI");
		expect(ownerOfElvUI).toBeNull();

		const ownerOfNonExistent = dbManager.getOwnerOf("NonExistent");
		expect(ownerOfNonExistent).toBeNull();
	});
});
