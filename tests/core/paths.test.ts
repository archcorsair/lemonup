import { describe, expect, mock, spyOn, test } from "bun:test";
import os from "node:os";
import { getDefaultWoWPath, isPathConfigured } from "@/core/paths";

describe("paths", () => {
	test("isPathConfigured should return false for NOT_CONFIGURED or empty", () => {
		expect(isPathConfigured("NOT_CONFIGURED")).toBe(false);
		expect(isPathConfigured("")).toBe(false);
		expect(isPathConfigured("/some/path")).toBe(true);
	});

	test("getDefaultWoWPath should return correct path for Windows", () => {
		spyOn(os, "platform").mockReturnValue("win32");
		expect(getDefaultWoWPath()).toContain("C:\\Program Files (x86)");
		mock.restore();
	});

	test("getDefaultWoWPath should return correct path for macOS", () => {
		spyOn(os, "platform").mockReturnValue("darwin");
		expect(getDefaultWoWPath()).toBe(
			"/Applications/World of Warcraft/_retail_/Interface/AddOns",
		);
		mock.restore();
	});

	test("getDefaultWoWPath should return Wine path for Linux", () => {
		spyOn(os, "platform").mockReturnValue("linux");

		const fs = require("node:fs");
		const originalExistsSync = fs.existsSync;
		// biome-ignore lint/suspicious/noExplicitAny: path can be string or URL
		spyOn(fs, "existsSync").mockImplementation((p: any) => {
			if (typeof p === "string" && p.includes(".wine")) return true;
			return false;
		});

		expect(getDefaultWoWPath()).toContain(".wine");

		mock.restore();
		fs.existsSync = originalExistsSync; // Restore manually just in case
	});
});
