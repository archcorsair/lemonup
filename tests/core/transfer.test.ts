import { describe, expect, test } from "bun:test";
import {
  ExportFileSchema,
  ExportedAddonSchema,
  exportAddons,
  parseImportFile,
  analyzeImport,
  type ExportFile,
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

describe("parseImportFile", () => {
  test("parses valid export file", async () => {
    const tmpPath = `/tmp/test-import-${Date.now()}.json`;
    const validFile = {
      version: 1,
      exportedAt: new Date().toISOString(),
      addons: [
        {
          name: "Test",
          folder: "Test",
          type: "github",
          url: "https://github.com/test/test",
          reinstallable: true,
        },
      ],
    };
    await Bun.write(tmpPath, JSON.stringify(validFile));

    const result = await parseImportFile(tmpPath);
    expect(result.success).toBe(true);
    expect(result.data?.addons.length).toBe(1);
  });

  test("fails on invalid JSON", async () => {
    const tmpPath = `/tmp/test-invalid-${Date.now()}.json`;
    await Bun.write(tmpPath, "not json");

    const result = await parseImportFile(tmpPath);
    expect(result.success).toBe(false);
    expect(result.error).toContain("parse");
  });

  test("fails on missing file", async () => {
    const result = await parseImportFile("/nonexistent/path.json");
    expect(result.success).toBe(false);
  });
});

describe("analyzeImport", () => {
  test("categorizes addons correctly", () => {
    const exportData: ExportFile = {
      version: 1,
      exportedAt: new Date().toISOString(),
      addons: [
        {
          name: "New",
          folder: "NewAddon",
          type: "github",
          url: "https://...",
          reinstallable: true,
        },
        {
          name: "Existing",
          folder: "ExistingAddon",
          type: "tukui",
          url: "https://...",
          reinstallable: true,
        },
        {
          name: "Manual",
          folder: "ManualAddon",
          type: "manual",
          url: null,
          reinstallable: false,
        },
      ],
    };

    const currentAddons: AddonRecord[] = [
      {
        name: "Existing",
        folder: "ExistingAddon",
        type: "tukui",
        url: "https://...",
        ownedFolders: [],
        kind: "addon",
        kindOverride: false,
        flavor: "retail",
        version: "1.0",
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

    const analysis = analyzeImport(exportData, currentAddons);

    expect(analysis.toInstall.length).toBe(1);
    expect(analysis.toInstall[0]?.folder).toBe("NewAddon");
    expect(analysis.alreadyInstalled.length).toBe(1);
    expect(analysis.manualAddons.length).toBe(1);
  });
});
