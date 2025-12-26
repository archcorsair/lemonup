import { Box, Text, useStdout } from "ink";
import Color from "ink-color-pipe";
import Gradient from "ink-gradient";
import Spinner from "ink-spinner";
import type React from "react";
import { useEffect, useState } from "react";
import { useTheme } from "@/tui/hooks/useTheme";
import { useAppStore } from "@/tui/store/useAppStore";
import pkg from "../../../package.json";

// Workaround for React 19 + Ink type mismatch
const SpinnerFixed = Spinner as unknown as React.FC<{
  type?: string;
}>;

const LOGO_TEXT = `
█   █▀▀ █▀▄▀█ █▀█ █▄ █ █ █ █▀█
█▄▄ ██▄ █ ▀ █ █▄█ █ ▀█ █▄█ █▀▀
`.trim();

interface HeaderProps {
  dryRun?: boolean;
  isBusy?: boolean;
  testMode?: boolean;
}

export const Header: React.FC<HeaderProps> = ({
  dryRun = false,
  isBusy = false,
  testMode = false,
}) => {
  const { theme } = useTheme();
  const pendingUpdates = useAppStore((state) => state.pendingUpdates);
  const isBackgroundChecking = useAppStore(
    (state) => state.isBackgroundChecking,
  );
  const nextCheckTime = useAppStore((state) => state.nextCheckTime);
  const { stdout } = useStdout();
  const [dims, setDims] = useState({
    cols: stdout.columns ?? 80,
    rows: stdout.rows ?? 24,
  });
  const [countdown, setCountdown] = useState<string | null>(null);

  useEffect(() => {
    const handler = () => {
      setDims({
        cols: stdout.columns ?? 80,
        rows: stdout.rows ?? 24,
      });
    };
    stdout.on("resize", handler);
    return () => {
      stdout.off("resize", handler);
    };
  }, [stdout]);

  // Countdown timer for dev mode
  useEffect(() => {
    if (!testMode || !nextCheckTime) {
      setCountdown(null);
      return;
    }

    const updateCountdown = () => {
      const remaining = Math.max(0, nextCheckTime - Date.now());
      const totalSeconds = Math.floor(remaining / 1000);
      const hours = Math.floor(totalSeconds / 3600);
      const minutes = Math.floor((totalSeconds % 3600) / 60);
      const seconds = totalSeconds % 60;
      setCountdown(
        `${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`,
      );
    };

    updateCountdown();
    const interval = setInterval(updateCountdown, 1000);
    return () => clearInterval(interval);
  }, [testMode, nextCheckTime]);

  return (
    <Box
      flexDirection="row"
      justifyContent="space-between"
      width="100%"
      alignItems="center"
    >
      <Box flexDirection="row">
        <Box alignItems="center">
          <Box marginBottom={1}>
            <Gradient name="fruit">
              <Text>{LOGO_TEXT}</Text>
            </Gradient>
          </Box>

          <Box marginLeft={1}>
            <Color styles={theme.version}>
              <Text>🍋 v{pkg.version}</Text>
            </Color>
          </Box>

          {isBackgroundChecking ? (
            <Box marginLeft={1}>
              <Color styles={theme.statusChecking}>
                <Text>
                  <SpinnerFixed type="dots" /> Checking...
                </Text>
              </Color>
            </Box>
          ) : (
            pendingUpdates > 0 && (
              <Box marginLeft={1}>
                <Color styles={theme.warning}>
                  <Text bold>
                    [{pendingUpdates} update{pendingUpdates > 1 ? "s" : ""}]
                  </Text>
                </Color>
              </Box>
            )
          )}
        </Box>

        {dryRun && (
          <Box marginLeft={2}>
            <Color styles={theme.dryRun}>
              <Text>[DRY RUN]</Text>
            </Color>
          </Box>
        )}
        {isBusy && (
          <Box marginLeft={2}>
            <Color styles={theme.busy}>
              <Text>
                <SpinnerFixed type="dots" /> Working...
              </Text>
            </Color>
          </Box>
        )}
      </Box>

      {testMode && (
        <Box gap={2}>
          <Color styles={theme.testMode}>
            <Text>
              DEV MODE ({dims.cols}x{dims.rows})
            </Text>
          </Color>
          {countdown && (
            <Color styles={theme.info}>
              <Text>Next check: {countdown}</Text>
            </Color>
          )}
        </Box>
      )}
    </Box>
  );
};
