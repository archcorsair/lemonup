import { useAppStore } from "@/tui/store/useAppStore";

/**
 * Hook to access the current theme and theme mode.
 */
export const useTheme = () => {
  const theme = useAppStore((state) => state.theme);
  const themeMode = useAppStore((state) => state.themeMode);
  const setTheme = useAppStore((state) => state.setTheme);

  return { theme, themeMode, setTheme };
};
