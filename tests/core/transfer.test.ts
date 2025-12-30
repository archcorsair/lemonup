import { describe, expect, test } from "bun:test";
import {
  ExportFileSchema,
  ExportedAddonSchema,
  exportAddons,
} from "@/core/transfer";
import type { AddonRecord } from "@/core/db";

describe("transfer schemas", () => {
  test("ExportedAddonSchema validates reinstallable addon", () => {
    const valid = {
      name: "ElvUI",
      folder: "ElvUI",
      type: "tukui",
      url: "https://api.tukui.org/...",
      reinstallable: true,
    };
    expect(() => ExportedAddonSchema.parse(valid)).not.toThrow();
  });

  test("ExportedAddonSchema validates manual addon", () => {
    const manual = {
      name: "MyAddon",
      folder: "MyAddon",
      type: "manual",
      url: null,
      reinstallable: false,
    };
    expect(() => ExportedAddonSchema.parse(manual)).not.toThrow();
  });

  test("ExportFileSchema validates complete export", () => {
    const file = {
      version: 1,
      exportedAt: new Date().toISOString(),
      addons: [],
    };
    expect(() => ExportFileSchema.parse(file)).not.toThrow();
  });
});

describe("exportAddons", () => {
  test("exports addons to JSON file", async () => {
    const mockAddons: AddonRecord[] = [
      {
        name: "ElvUI",
        folder: "ElvUI",
        type: "tukui",
        url: "https://tukui.org/elvui",
        ownedFolders: ["ElvUI_Options"],
        kind: "addon",
        kindOverride: false,
        flavor: "retail",
        version: "1.0",
        git_commit: null,
        author: "Elv",
        interface: "110000",
        requiredDeps: [],
        optionalDeps: [],
        embeddedLibs: [],
        install_date: "2024-01-01",
        last_updated: "2024-01-01",
      },
    ];

    const tmpPath = `/tmp/test-export-${Date.now()}.json`;
    const result = await exportAddons(mockAddons, tmpPath);

    expect(result.success).toBe(true);
    expect(result.count).toBe(1);

    const file = await Bun.file(tmpPath).json();
    expect(file.version).toBe(1);
    expect(file.addons[0].reinstallable).toBe(true);
  });

  test("marks manual addons as not reinstallable", async () => {
    const mockAddons: AddonRecord[] = [
      {
        name: "LocalAddon",
        folder: "LocalAddon",
        type: "manual",
        url: null,
        ownedFolders: [],
        kind: "addon",
        kindOverride: false,
        flavor: "retail",
        version: null,
        git_commit: null,
        author: null,
        interface: null,
        requiredDeps: [],
        optionalDeps: [],
        embeddedLibs: [],
        install_date: "2024-01-01",
        last_updated: "2024-01-01",
      },
    ];

    const tmpPath = `/tmp/test-export-manual-${Date.now()}.json`;
    await exportAddons(mockAddons, tmpPath);

    const file = await Bun.file(tmpPath).json();
    expect(file.addons[0].reinstallable).toBe(false);
  });
});
