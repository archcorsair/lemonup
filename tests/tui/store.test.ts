import { beforeEach, describe, expect, test } from "bun:test";
import { useAppStore } from "@/tui/store/useAppStore";
import { themes } from "@/tui/theme";

describe("App Store", () => {
  beforeEach(() => {
    // Reset store state
    useAppStore.setState({
      activeScreen: "menu",
      themeMode: "dark",
      theme: themes.dark,
    });
  });

  test("should initialize with default theme", () => {
    const state = useAppStore.getState();
    expect(state.themeMode).toBe("dark");
    expect(state.theme).toEqual(themes.dark);
  });

  test("setTheme should update themeMode and theme object", () => {
    useAppStore.getState().setTheme("light");
    
    const state = useAppStore.getState();
    expect(state.themeMode).toBe("light");
    expect(state.theme).toEqual(themes.light);
  });
});
