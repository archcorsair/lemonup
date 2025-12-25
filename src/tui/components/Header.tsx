import { Box, Text, useStdout } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { useEffect, useState } from "react";
import pkg from "../../../package.json";

// Workaround for React 19 + Ink type mismatch
const SpinnerFixed = Spinner as unknown as React.FC<{
  type?: string;
}>;

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
  const { stdout } = useStdout();
  const [dims, setDims] = useState({
    cols: stdout.columns ?? 80,
    rows: stdout.rows ?? 24,
  });

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

  return (
    <Box
      marginBottom={1}
      borderStyle="single"
      borderColor="blue"
      paddingX={1}
      minHeight={3}
    >
      <Box
        flexDirection="row"
        justifyContent="space-between"
        width="100%"
        alignItems="center"
      >
        <Box flexDirection="row">
          <Text bold color="cyan">
            LemonUp 🍋
          </Text>
          <Box marginLeft={1}>
            <Text color="gray">v{pkg.version}</Text>
          </Box>
          {dryRun && (
            <Box marginLeft={2}>
              <Text color="yellow" bold>
                [DRY RUN]
              </Text>
            </Box>
          )}
          {isBusy && (
            <Box marginLeft={2}>
              <Text color="yellow">
                <SpinnerFixed type="dots" /> Working...
              </Text>
            </Box>
          )}
        </Box>

        {testMode && (
          <Box>
            <Text color="magenta">
              TEST MODE ({dims.cols}x{dims.rows})
            </Text>
          </Box>
        )}
      </Box>
    </Box>
  );
};
