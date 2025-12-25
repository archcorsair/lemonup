import { create } from "zustand";

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

  // Key Feedback State
  activeKey: string | null;

  // Toast State
  toast: Toast | null;

  // Actions
  navigate: (screen: Screen) => void;
  setLastMenuSelection: (selection: string) => void;
  setBusy: (busy: boolean) => void;
  flashKey: (key: string) => void;
  showToast: (message: string, duration?: number) => void;
  clearToast: () => void;
}

let toastId = 0;

export const useAppStore = create<AppState>((set, get) => ({
  activeScreen: "menu",
  lastMenuSelection: null, // default to null so MainMenu uses config default
  isBusy: false,
  activeKey: null,
  toast: null,

  navigate: (screen) => set({ activeScreen: screen }),
  setLastMenuSelection: (selection) => set({ lastMenuSelection: selection }),
  setBusy: (busy) => set({ isBusy: busy }),
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
}));
