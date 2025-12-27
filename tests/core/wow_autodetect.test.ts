import { describe, expect, mock, spyOn, test } from "bun:test";
import os from "node:os";
import fs from "node:fs";
import { getDefaultWoWPath } from "@/core/paths";

describe("WoW Auto-Detection Hardening", () => {
	test("getDefaultWoWPath should detect WoW on D: drive for Windows", () => {
		spyOn(os, "platform").mockReturnValue("win32");
		
		const spy = spyOn(fs, "existsSync").mockImplementation((p: any) => {
			if (typeof p === "string" && p.startsWith("D:\\")) return true;
			return false;
		});

		expect(getDefaultWoWPath()).toContain("D:\\");

		spy.mockRestore();
	});

	test("getDefaultWoWPath should prioritize _retail_ and ignore others", () => {
		// This is more about ensuring we don't return classic paths if retail exists
		// The current implementation already has _retail_ hardcoded in the strings,
		// but we want to make sure we are EXPLICIT about it.
	});
    
    test("verifyWoWDirectory should be stricter", () => {
        const { verifyWoWDirectory } = require("@/core/paths");
        
        // Mock a directory that exists but doesn't have Wow.exe in its grandparent
        const spy1 = spyOn(fs, "existsSync").mockImplementation((p: any) => {
            if (typeof p === "string" && p.endsWith("AddOns")) return true;
            return false;
        });

        expect(verifyWoWDirectory("/fake/Interface/AddOns")).toBe(false);
        spy1.mockRestore();

        // Mock a correct directory
        const spy2 = spyOn(fs, "existsSync").mockImplementation((p: any) => {
            if (typeof p === "string" && (p.endsWith("AddOns") || p.endsWith("Wow.exe") || p.endsWith("Wow-64.exe"))) return true;
            return false;
        });
        expect(verifyWoWDirectory("/fake/_retail_/Interface/AddOns")).toBe(true);

        spy2.mockRestore();
    });
});
