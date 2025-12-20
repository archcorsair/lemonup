import { create } from "zustand";

export type Screen = "menu" | "update" | "manage" | "config" | "install";

interface AppState {
	// Navigation State
	activeScreen: Screen;
	lastMenuSelection: string;

	// Global UI State
	isBusy: boolean;

	// Key Feedback State
	activeKey: string | null;

	// Actions
	navigate: (screen: Screen) => void;
	setLastMenuSelection: (selection: string) => void;
	setBusy: (busy: boolean) => void;
	flashKey: (key: string) => void;
}

export const useAppStore = create<AppState>((set) => ({
	activeScreen: "menu",
	lastMenuSelection: "update", // default fallback
	isBusy: false,
	activeKey: null,

	navigate: (screen) => set({ activeScreen: screen }),
	setLastMenuSelection: (selection) => set({ lastMenuSelection: selection }),
	setBusy: (busy) => set({ isBusy: busy }),
	flashKey: (key) => {
		set({ activeKey: key });
		setTimeout(() => {
			set((state) => (state.activeKey === key ? { activeKey: null } : {}));
		}, 150);
	},
}));
