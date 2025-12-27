import { describe, expect, mock, spyOn, test } from "bun:test";
import os from "node:os";
import fs from "node:fs";
import { getDefaultWoWPath, isPathConfigured } from "@/core/paths";

describe("paths", () => {
	test("isPathConfigured should return false for NOT_CONFIGURED or empty", () => {
		expect(isPathConfigured("NOT_CONFIGURED")).toBe(false);
		expect(isPathConfigured("")).toBe(false);
		expect(isPathConfigured("/some/path")).toBe(true);
	});

	test("getDefaultWoWPath should return correct path for Windows", () => {
		spyOn(os, "platform").mockReturnValue("win32");
		const spy = spyOn(fs, "existsSync").mockReturnValue(true);
		expect(getDefaultWoWPath()).toContain("C:\\Program Files (x86)");
		spy.mockRestore();
		mock.restore();
	});

	test("getDefaultWoWPath should return correct path for macOS", () => {
		spyOn(os, "platform").mockReturnValue("darwin");
		const spy = spyOn(fs, "existsSync").mockReturnValue(true);
		expect(getDefaultWoWPath()).toBe(
			"/Applications/World of Warcraft/_retail_/Interface/AddOns",
		);
		spy.mockRestore();
		mock.restore();
	});

	test("getDefaultWoWPath should return Wine path for Linux", () => {
		spyOn(os, "platform").mockReturnValue("linux");

		const originalExistsSync = fs.existsSync;
		spyOn(fs, "existsSync").mockImplementation((p: any) => {
			if (typeof p === "string" && p.includes(".wine")) return true;
			return false;
		});

		expect(getDefaultWoWPath()).toContain(".wine");

		mock.restore();
		fs.existsSync = originalExistsSync; // Restore manually just in case
	});

	test("getDefaultWoWPath checks Program Files and Games on Windows", () => {
		spyOn(os, "platform").mockReturnValue("win32");

		// Mock: Only D:\Games\World of Warcraft exists
		const spy = spyOn(fs, "existsSync").mockImplementation((p: any) => {
			if (typeof p !== "string") return false;
			if (p.includes("D:\\Games\\World of Warcraft\\_retail_")) return true;
			if (p === "D:\\Games\\World of Warcraft\\_retail_\\Interface\\AddOns")
				return true;
			// Need artifacts for verification
			if (p === "D:\\Games\\World of Warcraft\\_retail_\\Wow.exe") return true;
			if (p === "D:\\Games\\World of Warcraft\\_retail_\\Data") return true;
			return false;
		});

		const result = getDefaultWoWPath();
		expect(result).toContain("D:\\Games");

		spy.mockRestore();
		mock.restore();
	});
});
