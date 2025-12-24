import { beforeEach, describe, expect, test } from "bun:test";
import { useAppStore } from "@/tui/store/useAppStore";

describe("Toast", () => {
	beforeEach(() => {
		useAppStore.setState({ toast: null });
	});

	test("showToast sets toast message", () => {
		useAppStore.getState().showToast("Test message");
		expect(useAppStore.getState().toast?.message).toBe("Test message");
	});

	test("showToast with duration=0 does not auto-clear", async () => {
		useAppStore.getState().showToast("Persistent", 0);
		await new Promise((r) => setTimeout(r, 100));
		expect(useAppStore.getState().toast?.message).toBe("Persistent");
	});

	test("new toast supersedes old toast", () => {
		useAppStore.getState().showToast("First");
		const firstId = useAppStore.getState().toast?.id;
		useAppStore.getState().showToast("Second");
		expect(useAppStore.getState().toast?.message).toBe("Second");
		expect(useAppStore.getState().toast?.id).not.toBe(firstId);
	});

	test("old timeout does not clear new toast", async () => {
		useAppStore.getState().showToast("First", 50);
		useAppStore.getState().showToast("Second", 200);
		await new Promise((r) => setTimeout(r, 100));
		// First timeout fired but should not have cleared Second
		expect(useAppStore.getState().toast?.message).toBe("Second");
	});

	test("clearToast removes toast", () => {
		useAppStore.getState().showToast("Test");
		useAppStore.getState().clearToast();
		expect(useAppStore.getState().toast).toBeNull();
	});
});
