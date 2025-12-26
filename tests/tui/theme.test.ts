import { expect, test, describe } from "bun:test";
// @ts-ignore - This file doesn't exist yet, but we are writing the test first
import { THEME, DARK_THEME, LIGHT_THEME, themes } from "@/tui/theme";

describe("Theme Registry", () => {
  test("exports legacy THEME object", () => {
    expect(THEME).toBeDefined();
    expect(typeof THEME).toBe("object");
  });

  test("exports DARK_THEME and LIGHT_THEME", () => {
    expect(DARK_THEME).toBeDefined();
    expect(LIGHT_THEME).toBeDefined();
  });

  test("themes have consistent keys", () => {
    const darkKeys = Object.keys(DARK_THEME).sort();
    const lightKeys = Object.keys(LIGHT_THEME).sort();
    expect(darkKeys).toEqual(lightKeys);
  });

  test("themes map contains both", () => {
    expect(themes.dark).toBe(DARK_THEME);
    expect(themes.light).toBe(LIGHT_THEME);
  });

  test("defines core semantic styles", () => {
    expect(THEME.brand).toBeDefined();
    expect(THEME.success).toBeDefined();
    expect(THEME.error).toBeDefined();
    expect(THEME.warning).toBeDefined();
    expect(THEME.muted).toBeDefined();
    expect(THEME.highlight).toBeDefined();
  });
});
