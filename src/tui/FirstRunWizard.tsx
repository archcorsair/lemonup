import { Box, Text, useApp, useInput } from "ink";
import type React from "react";
import { useState } from "react";
import type { ConfigManager } from "@/core/config";

interface FirstRunWizardProps {
  configManager: ConfigManager;
  onComplete: () => void;
}

export const FirstRunWizard: React.FC<FirstRunWizardProps> = ({
  configManager,
  onComplete,
}) => {
  const { exit } = useApp();
  const [error, setError] = useState<string | null>(null);

  useInput((input, key) => {
    if (input.toLowerCase() === "y" || key.return) {
      try {
        configManager.createDefaultConfig();
        onComplete();
      } catch (e) {
        setError(String(e));
      }
    } else if (input.toLowerCase() === "n" || key.escape) {
      exit();
    }
  });

  return (
    <Box
      flexDirection="column"
      padding={1}
      borderStyle="round"
      borderColor="cyan"
    >
      <Text bold color="yellow">
        Configuration Not Found
      </Text>
      <Text>
        No configuration file found at:
        <Text color="blue"> {configManager.path}</Text>
      </Text>
      <Box marginTop={1}>
        <Text>
          Would you like to create a default configuration template? (Y/n)
        </Text>
      </Box>
      {error && (
        <Box marginTop={1}>
          <Text color="red">Error creating config: {error}</Text>
        </Box>
      )}
    </Box>
  );
};
