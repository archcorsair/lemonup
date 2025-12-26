import { create } from "zustand";
import { type Theme, themes } from "@/tui/theme";

export type Screen = "menu" | "update" | "manage" | "config" | "install";

interface Toast {
  message: string;
  id: number;
}

interface AppState {
  // Navigation State
  activeScreen: Screen;
  lastMenuSelection: string | null;

  // Global UI State
  isBusy: boolean;
  theme: Theme;
  themeMode: "dark" | "light";

  // Key Feedback State
  activeKey: string | null;

  // Toast State
  toast: Toast | null;

  // Background Update State
  pendingUpdates: number;
  isBackgroundChecking: boolean;
  nextCheckTime: number | null; // timestamp of next scheduled check

  // Dev Mode
  devMode: boolean;

  // Actions
  navigate: (screen: Screen) => void;
  setLastMenuSelection: (selection: string) => void;
  setBusy: (busy: boolean) => void;
  setTheme: (mode: "dark" | "light") => void;
  flashKey: (key: string) => void;
  showToast: (message: string, duration?: number) => void;
  clearToast: () => void;
  setPendingUpdates: (count: number) => void;
  setBackgroundChecking: (checking: boolean) => void;
  setNextCheckTime: (time: number | null) => void;
  setDevMode: (enabled: boolean) => void;
}

let toastId = 0;

export const useAppStore = create<AppState>((set, get) => ({
  activeScreen: "menu",
  lastMenuSelection: null, // default to null so MainMenu uses config default
  isBusy: false,
  theme: themes.dark,
  themeMode: "dark",
  activeKey: null,
  toast: null,
  pendingUpdates: 0,
  isBackgroundChecking: false,
  nextCheckTime: null,
  devMode: false,

  navigate: (screen) => set({ activeScreen: screen }),
  setLastMenuSelection: (selection) => set({ lastMenuSelection: selection }),
  setBusy: (busy) => set({ isBusy: busy }),
  setTheme: (mode) => set({ themeMode: mode, theme: themes[mode] }),
  flashKey: (key) => {
    set({ activeKey: key });
    setTimeout(() => {
      set((state) => (state.activeKey === key ? { activeKey: null } : {}));
    }, 150);
  },
  showToast: (message, duration = 3000) => {
    const id = ++toastId;
    set({ toast: { message, id } });

    if (duration > 0) {
      setTimeout(() => {
        // Only clear if this is still the active toast
        if (get().toast?.id === id) {
          set({ toast: null });
        }
      }, duration);
    }
  },
  clearToast: () => set({ toast: null }),
  setPendingUpdates: (count) => set({ pendingUpdates: count }),
  setBackgroundChecking: (checking) => set({ isBackgroundChecking: checking }),
  setNextCheckTime: (time) => set({ nextCheckTime: time }),
  setDevMode: (enabled) => set({ devMode: enabled }),
}));
