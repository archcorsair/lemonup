import { describe, expect, mock, spyOn, test } from "bun:test";
import os from "node:os";
import { getDefaultWoWPath, isPathConfigured } from "./paths";

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
		expect(getDefaultWoWPath()).toContain(".wine");
		mock.restore();
	});
});
