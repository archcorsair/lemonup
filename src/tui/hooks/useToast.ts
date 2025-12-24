import { useAppStore } from "@/tui/store/useAppStore";

export function useToast() {
	const toast = useAppStore((state) => state.toast);
	const showToast = useAppStore((state) => state.showToast);
	const clearToast = useAppStore((state) => state.clearToast);

	return { toast, showToast, clearToast };
}
