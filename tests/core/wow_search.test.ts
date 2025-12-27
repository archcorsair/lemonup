import { describe, expect, mock, spyOn, test } from "bun:test";
import fs from "node:fs";
import path from "node:path";
import { searchForWoW } from "@/core/paths";

describe("WoW Deep Search", () => {
  test("should find WoW Retail in a deep directory structure", async () => {
    const root = "/home/user";
    const target = "/home/user/Games/WoW/_retail_/Interface/AddOns";

    const spy = spyOn(fs, "readdirSync").mockImplementation((p: any) => {
      if (p === "/home/user") return ["Documents", "Games"] as any;
      if (p === "/home/user/Games") return ["WoW"] as any;
      if (p === "/home/user/Games/WoW") return ["_retail_"] as any;
      if (p === "/home/user/Games/WoW/_retail_") return ["Interface", "Wow.exe", "Data"] as any;
      if (p === "/home/user/Games/WoW/_retail_/Interface") return ["AddOns"] as any;
      return [] as any;
    });

    const statSpy = spyOn(fs, "statSync").mockImplementation((p: any) => {
      return {
        isDirectory: () => !p.endsWith("Wow.exe"),
      } as any;
    });

    const existsSpy = spyOn(fs, "existsSync").mockImplementation((p: any) => {
      if (p === target) return true;
      if (p.includes("Wow.exe")) return true;
      if (p.includes("Data")) return true; // Add second artifact
      return false;
    });

    const result = await searchForWoW(root);
    expect(result).toBe(target);

    spy.mockRestore();
    statSpy.mockRestore();
    existsSpy.mockRestore();
  });

  test("should ignore irrelevant directories", async () => {
    const root = "/home/user";
    
    const readdirSpy = spyOn(fs, "readdirSync").mockImplementation((p: any) => {
      if (p === "/home/user") return ["node_modules", ".git", "Library"] as any;
      return [] as any;
    });

    const statSpy = spyOn(fs, "statSync").mockImplementation((p: any) => {
      return { isDirectory: () => true } as any;
    });

    await searchForWoW(root);
    
    // Should NOT have searched inside node_modules or .git
    expect(readdirSpy).toHaveBeenCalledTimes(1); 
    expect(readdirSpy).toHaveBeenCalledWith("/home/user");

    readdirSpy.mockRestore();
    statSpy.mockRestore();
  });

  test("should be interruptible", async () => {
    const root = "/home/user";
    const controller = new AbortController();

    const readdirSpy = spyOn(fs, "readdirSync").mockImplementation((p: any) => {
      // On the first call, we abort
      controller.abort();
      return ["folder1", "folder2"] as any;
    });

    const statSpy = spyOn(fs, "statSync").mockImplementation((p: any) => {
      return { isDirectory: () => true } as any;
    });

    const result = await searchForWoW(root, controller.signal);
    expect(result).toBeNull();

    // Should have stopped immediately after the first read (which triggered abort)
    // Actually the queue shift happens after the signal check in the loop,
    // so it might do one iteration.
    expect(readdirSpy).toHaveBeenCalledTimes(1);

    readdirSpy.mockRestore();
    statSpy.mockRestore();
  });

  test("ignores Windows and Linux system directories", async () => {
    const root = "/home/user";
    let checkedDirs: string[] = [];

    const spy = spyOn(fs, "readdirSync").mockImplementation((p: any) => {
      checkedDirs.push(p);
      if (p === "/home/user") {
        return ["Windows", "tmp", "var", "Games"] as any;
      }
      return [] as any;
    });

    const statSpy = spyOn(fs, "statSync").mockReturnValue({
      isDirectory: () => true,
    } as any);

    await searchForWoW(root);

    expect(checkedDirs).toContain("/home/user");
    expect(checkedDirs).not.toContain("/home/user/Windows");
    expect(checkedDirs).not.toContain("/home/user/tmp");

    spy.mockRestore();
    statSpy.mockRestore();
  });

  test("calls progress callback during scan", async () => {
    const root = "/home/user";
    let progressCalls = 0;

    const spy = spyOn(fs, "readdirSync").mockImplementation((p: any) => {
      if (p === "/home/user") return ["folder1", "folder2"] as any;
      return [] as any;
    });

    const statSpy = spyOn(fs, "statSync").mockReturnValue({
      isDirectory: () => true,
    } as any);

    await searchForWoW(
      root,
      undefined,
      (dirsScanned, currentPath) => {
        progressCalls++;
        expect(dirsScanned).toBeGreaterThan(0);
        expect(typeof currentPath).toBe("string");
      },
    );

    expect(progressCalls).toBeGreaterThan(0);

    spy.mockRestore();
    statSpy.mockRestore();
  });
});
