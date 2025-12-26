import { useCallback, useMemo, useRef } from "react";
import {
  clearTerminalProgress,
  setTerminalProgress,
  TerminalProgressState,
} from "@/core/terminal";

interface ProgressBarOptions {
  enabled?: boolean; // Whether terminal progress bar is enabled (default: true)
  minDuration?: number; // Minimum total duration (default: 2000ms)
  holdDuration?: number; // Hold at 100% before clearing (default: 500ms)
  warningDuration?: number; // Warning flash duration (default: 200ms)
}

interface ProgressBarControls {
  /** Start progress tracking for N total steps */
  start: (totalSteps: number) => void;
  /** Advance one step, ensuring minimum step duration */
  advance: () => Promise<void>;
  /** Flash warning state briefly, then continue */
  warn: () => Promise<void>;
  /** Complete and clear the progress bar */
  complete: () => Promise<void>;
  /** Clear immediately (for cleanup/abort) */
  clear: () => void;
}

export const useProgressBar = (
  options: ProgressBarOptions = {},
): ProgressBarControls => {
  const {
    enabled = true,
    minDuration = 2000,
    holdDuration = 500,
    warningDuration = 200,
  } = options;

  const stateRef = useRef({
    totalSteps: 0,
    currentStep: 0,
    stepStartTime: 0,
    minTimePerStep: 0,
  });

  const start = useCallback(
    (totalSteps: number) => {
      stateRef.current = {
        totalSteps,
        currentStep: 0,
        stepStartTime: Date.now(),
        minTimePerStep: minDuration / totalSteps,
      };
      if (enabled) {
        setTerminalProgress(TerminalProgressState.Running, 0);
      }
    },
    [minDuration, enabled],
  );

  const advance = useCallback(async () => {
    const state = stateRef.current;
    const elapsed = Date.now() - state.stepStartTime;
    const remaining = state.minTimePerStep - elapsed;

    if (enabled && remaining > 0) {
      await new Promise((r) => setTimeout(r, remaining));
    }

    state.currentStep++;
    if (enabled) {
      const progress = (state.currentStep / state.totalSteps) * 100;
      setTerminalProgress(TerminalProgressState.Running, progress);
    }
    state.stepStartTime = Date.now();
  }, [enabled]);

  const warn = useCallback(async () => {
    if (!enabled) return;
    const state = stateRef.current;
    const progress = ((state.currentStep + 1) / state.totalSteps) * 100;
    setTerminalProgress(TerminalProgressState.Warning, progress);
    await new Promise((r) => setTimeout(r, warningDuration));
  }, [warningDuration, enabled]);

  const complete = useCallback(async () => {
    if (!enabled) return;
    setTerminalProgress(TerminalProgressState.Running, 100);
    await new Promise((r) => setTimeout(r, holdDuration));
    clearTerminalProgress();
  }, [holdDuration, enabled]);

  const clear = useCallback(() => {
    if (!enabled) return;
    clearTerminalProgress();
  }, [enabled]);

  return useMemo(
    () => ({ start, advance, warn, complete, clear }),
    [start, advance, warn, complete, clear],
  );
};
