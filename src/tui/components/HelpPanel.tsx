import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import type React from "react";
import { THEME } from "../theme";

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
  if (!expanded) return null;

  return (
    <Box
      borderStyle="single"
      borderColor={THEME.border}
      borderTop={false}
      paddingX={1}
      flexWrap="wrap"
    >
      {shortcuts.map((s) => (
        <Box key={s.key} marginRight={2}>
          <Color styles={THEME.helpKey}>
            <Text>{s.key}</Text>
          </Color>
          <Color styles={THEME.helpLabel}>
            <Text> {s.label}</Text>
          </Color>
        </Box>
      ))}
    </Box>
  );
};
