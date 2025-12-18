export enum TerminalProgressState {
	Remove = 0,
	Running = 1,
	Error = 2,
	Paused = 3,
	Warning = 4,
}

/**
 * Sets the terminal progress bar using OSC 9;4 sequences.
 * Supported by Ghostty, WezTerm, iTerm2, etc.
 * Format: OSC 9 ; 4 ; <state> ; <progress> ST
 */
export const setTerminalProgress = (
	state: TerminalProgressState,
	progress = 0,
) => {
	// ST (String Terminator) can be \x07 (BEL) or \x1b\\ (ESC \)
	// Using \x1b\\ is safer for some terminals, but \x07 is common.
	// User example used standard OSC closing. using \x1b\\ (ESC \) is standard ST.
	const ST = "\x1b\\";
	const OSC = "\x1b]";

	// Ensure progress is 0-100
	const p = Math.max(0, Math.min(100, Math.round(progress)));

	const seq = `${OSC}9;4;${state};${p}${ST}`;
	process.stdout.write(seq);
};

export const clearTerminalProgress = () => {
	setTerminalProgress(TerminalProgressState.Remove);
};
