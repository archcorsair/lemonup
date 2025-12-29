import { describe, expect, test } from "bun:test";
import { ExportFileSchema, ExportedAddonSchema } from "@/core/transfer";

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
