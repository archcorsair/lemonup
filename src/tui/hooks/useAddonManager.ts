import { useEffect, useRef } from "react";
import type { AddonManagerEvents } from "../../core/events";
import type { AddonManager } from "../../core/manager";

export function useAddonManagerEvent<E extends keyof AddonManagerEvents>(
	manager: AddonManager | null,
	event: E,
	callback: (...args: AddonManagerEvents[E]) => void,
) {
	const callbackRef = useRef(callback);

	useEffect(() => {
		callbackRef.current = callback;
	}, [callback]);

	useEffect(() => {
		if (!manager) return;

		const handler = (...args: AddonManagerEvents[E]) => {
			callbackRef.current(...args);
		};

		manager.on(event, handler as any);

		return () => {
			manager.off(event, handler as any);
		};
	}, [manager, event]);
}
