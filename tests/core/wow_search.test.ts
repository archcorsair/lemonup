import { describe, expect, spyOn, test } from "bun:test";
import fs from "node:fs";
import type { Dirent } from "node:fs";
import { searchForWoW } from "@/core/paths";

// Helper to create mock Dirent objects
function mockDirent(name: string, isDir: boolean): Dirent {
	return {
		name,
		isDirectory: () => isDir,
		isFile: () => !isDir,
		isBlockDevice: () => false,
		isCharacterDevice: () => false,
		isFIFO: () => false,
		isSocket: () => false,
		isSymbolicLink: () => false,
		path: "",
		parentPath: "",
	} as Dirent;
}

describe("WoW Deep Search", () => {
	test("should find WoW Retail in a deep directory structure", async () => {
		const root = "/home/user";
		const target = "/home/user/Games/WoW/_retail_/Interface/AddOns";

		const accessSpy = spyOn(fs, "accessSync").mockImplementation(() => {});

		const spy = spyOn(fs, "readdirSync").mockImplementation(((p: string) => {
			if (p === "/home/user")
				return [mockDirent("Documents", true), mockDirent("Games", true)];
			if (p === "/home/user/Games") return [mockDirent("WoW", true)];
			if (p === "/home/user/Games/WoW") return [mockDirent("_retail_", true)];
			if (p === "/home/user/Games/WoW/_retail_")
				return [
					mockDirent("Interface", true),
					mockDirent("Wow.exe", false),
					mockDirent("Data", true),
				];
			if (p === "/home/user/Games/WoW/_retail_/Interface")
				return [mockDirent("AddOns", true)];
			return [];
		}) as unknown as typeof fs.readdirSync);

		const lstatSpy = spyOn(fs, "lstatSync").mockImplementation(() => {
			return {
				isSymbolicLink: () => false,
			} as any;
		});

		const existsSpy = spyOn(fs, "existsSync").mockImplementation((p: any) => {
			if (p === target) return true;
			if (p.includes("Wow.exe")) return true;
			if (p.includes("Data")) return true;
			return false;
		});

		const result = await searchForWoW(root);
		expect(result).toBe(target);

		accessSpy.mockRestore();
		spy.mockRestore();
		lstatSpy.mockRestore();
		existsSpy.mockRestore();
	});

	test("should ignore irrelevant directories", async () => {
		const root = "/home/user";

		const accessSpy = spyOn(fs, "accessSync").mockImplementation(() => {});

		const readdirSpy = spyOn(fs, "readdirSync").mockImplementation(((
			p: string,
		) => {
			if (p === "/home/user")
				return [
					mockDirent("node_modules", true),
					mockDirent(".git", true),
					mockDirent("Library", true),
				];
			return [];
		}) as unknown as typeof fs.readdirSync);

		const lstatSpy = spyOn(fs, "lstatSync").mockImplementation(() => {
			return {
				isSymbolicLink: () => false,
			} as any;
		});

		await searchForWoW(root);

		// Should have only read the root - ignored dirs should not be traversed
		expect(readdirSpy).toHaveBeenCalledWith("/home/user", {
			withFileTypes: true,
		});
		// All entries are in IGNORED_DIRS so no subdirs should be read
		expect(readdirSpy).toHaveBeenCalledTimes(1);

		accessSpy.mockRestore();
		readdirSpy.mockRestore();
		lstatSpy.mockRestore();
	});

	test("should be interruptible", async () => {
		const root = "/home/user";
		const controller = new AbortController();

		const accessSpy = spyOn(fs, "accessSync").mockImplementation(() => {});

		const readdirSpy = spyOn(fs, "readdirSync").mockImplementation(((
			p: string,
		) => {
			// Return entries, then abort after first call
			if (p === "/home/user") {
				return [mockDirent("folder1", true), mockDirent("folder2", true)];
			}
			// Abort on second call
			controller.abort();
			return [];
		}) as unknown as typeof fs.readdirSync);

		const lstatSpy = spyOn(fs, "lstatSync").mockImplementation(() => {
			return {
				isSymbolicLink: () => false,
			} as any;
		});

		const result = await searchForWoW(root, controller.signal);
		expect(result).toBeNull();

		accessSpy.mockRestore();
		readdirSpy.mockRestore();
		lstatSpy.mockRestore();
	});

	test("ignores Windows and Linux system directories", async () => {
		const root = "/home/user";
		const checkedDirs: string[] = [];

		const accessSpy = spyOn(fs, "accessSync").mockImplementation(() => {});

		const spy = spyOn(fs, "readdirSync").mockImplementation(((p: string) => {
			checkedDirs.push(p);
			if (p === "/home/user") {
				return [
					mockDirent("Windows", true),
					mockDirent("tmp", true),
					mockDirent("var", true),
					mockDirent("Games", true),
				];
			}
			return [];
		}) as unknown as typeof fs.readdirSync);

		const lstatSpy = spyOn(fs, "lstatSync").mockImplementation(() => {
			return {
				isSymbolicLink: () => false,
			} as any;
		});

		await searchForWoW(root);

		expect(checkedDirs).toContain("/home/user");
		// Windows is in IGNORED_DIRS, tmp is in IGNORED_DIRS, but Games is not
		expect(checkedDirs).not.toContain("/home/user/Windows");
		expect(checkedDirs).not.toContain("/home/user/tmp");
		expect(checkedDirs).toContain("/home/user/Games");

		accessSpy.mockRestore();
		spy.mockRestore();
		lstatSpy.mockRestore();
	});

	test("calls progress callback during scan", async () => {
		const root = "/home/user";
		let progressCalls = 0;

		const accessSpy = spyOn(fs, "accessSync").mockImplementation(() => {});

		const spy = spyOn(fs, "readdirSync").mockImplementation(((p: string) => {
			if (p === "/home/user")
				return [mockDirent("folder1", true), mockDirent("folder2", true)];
			return [];
		}) as unknown as typeof fs.readdirSync);

		const lstatSpy = spyOn(fs, "lstatSync").mockImplementation(() => {
			return {
				isSymbolicLink: () => false,
			} as any;
		});

		await searchForWoW(root, undefined, (dirsScanned, currentPath) => {
			progressCalls++;
			expect(dirsScanned).toBeGreaterThan(0);
			expect(typeof currentPath).toBe("string");
		});

		expect(progressCalls).toBeGreaterThan(0);

		accessSpy.mockRestore();
		spy.mockRestore();
		lstatSpy.mockRestore();
	});

	test("skips symbolic links to prevent loops", async () => {
		const root = "/home/user";
		const checkedDirs: string[] = [];

		const accessSpy = spyOn(fs, "accessSync").mockImplementation(() => {});

		const spy = spyOn(fs, "readdirSync").mockImplementation(((p: string) => {
			checkedDirs.push(p);
			if (p === "/home/user") {
				return [mockDirent("symlink", true), mockDirent("real", true)];
			}
			return [];
		}) as unknown as typeof fs.readdirSync);

		const lstatSpy = spyOn(fs, "lstatSync").mockImplementation((p: any) => {
			return {
				isSymbolicLink: () => (p as string).includes("symlink"),
			} as any;
		});

		await searchForWoW(root);

		// symlink should be skipped, real should be checked
		expect(checkedDirs).toContain("/home/user");
		expect(checkedDirs).not.toContain("/home/user/symlink");
		expect(checkedDirs).toContain("/home/user/real");

		accessSpy.mockRestore();
		spy.mockRestore();
		lstatSpy.mockRestore();
	});
});
