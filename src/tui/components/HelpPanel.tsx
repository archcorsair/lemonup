import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import type React from "react";
import { useTheme } from "@/tui/hooks/useTheme";

interface Shortcut {
  key: string;
  label: string;
}

interface HelpPanelProps {
  shortcuts: Shortcut[];
  expanded: boolean;
}

export const HelpPanel: React.FC<HelpPanelProps> = ({
  shortcuts,
  expanded,
}) => {
  const { theme } = useTheme();
  if (!expanded) return null;

  return (
    <Box
      borderStyle="single"
      borderColor={theme.border}
      borderTop={false}
      paddingX={1}
      flexWrap="wrap"
    >
      {shortcuts.map((s) => (
        <Box key={s.key} marginRight={2}>
          <Color styles={theme.helpKey}>
            <Text>{s.key}</Text>
          </Color>
          <Color styles={theme.helpLabel}>
            <Text> {s.label}</Text>
          </Color>
        </Box>
      ))}
    </Box>
  );
};
