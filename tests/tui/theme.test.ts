import { expect, test, describe } from "bun:test";
// @ts-ignore - This file doesn't exist yet, but we are writing the test first
import { THEME } from "@/tui/theme";

describe("Theme Registry", () => {
  test("exports a THEME object", () => {
    expect(THEME).toBeDefined();
    expect(typeof THEME).toBe("object");
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
