import { Box, Text } from "ink";
import type React from "react";

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
      borderColor="gray"
      borderTop={false}
      paddingX={1}
      flexWrap="wrap"
    >
      {shortcuts.map((s) => (
        <Box key={s.key} marginRight={2}>
          <Text color="yellow" bold>
            {s.key}
          </Text>
          <Text color="gray"> {s.label}</Text>
        </Box>
      ))}
    </Box>
  );
};
