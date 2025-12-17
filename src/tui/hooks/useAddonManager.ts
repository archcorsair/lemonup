import { useEffect, useState } from "react";
import type { AddonManager } from "../../core/manager";
import type { AddonManagerEvents } from "../../core/events";

export function useAddonManagerEvent<E extends keyof AddonManagerEvents>(
	manager: AddonManager | null,
	event: E,
	callback: (...args: AddonManagerEvents[E]) => void,
) {
	useEffect(() => {
		if (!manager) return;

		// @ts-ignore - EventEmitter types can be tricky with custom maps
		manager.on(event, callback);

		return () => {
			// @ts-ignore
			manager.off(event, callback);
		};
	}, [manager, event, callback]);
}
