export enum TerminalProgressState {
  Remove = 0,
  Running = 1,
  Error = 2,
  Paused = 3,
  Warning = 4,
}

// ANSI Escape Sequence Constants
const ESC = "\x1b";
const OSC = `${ESC}]`;
const ST = `${ESC}\\`;

/**
 * Sets the terminal progress bar using OSC 9;4 sequences.
 * Supported by Ghostty, WezTerm, iTerm2, etc.
 * Format: OSC 9 ; 4 ; <state> ; <progress> ST
 */
export const setTerminalProgress = (
  state: TerminalProgressState,
  progress = 0,
) => {
  if (!process.stdout.isTTY) return;

  // Ensure progress is 0-100
  const p = Math.max(0, Math.min(100, Math.round(progress)));

  const seq = `${OSC}9;4;${state};${p}${ST}`;
  process.stdout.write(seq);
};

export const clearTerminalProgress = () => {
  setTerminalProgress(TerminalProgressState.Remove);
};
