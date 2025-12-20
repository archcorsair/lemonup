import type { AddonManagerEvents } from "@/core/events";

export interface CommandContext {
	emit<E extends keyof AddonManagerEvents>(
		event: E,
		...args: AddonManagerEvents[E]
	): void;
}

export interface Command<T = void> {
	execute(context: CommandContext): Promise<T>;
	undo?(context: CommandContext): Promise<void>;
}
