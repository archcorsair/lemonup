import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import React from "react";
import { useAppStore } from "@/tui/store/useAppStore";
import { useTheme } from "../hooks/useTheme";

export interface ControlHelp {
  key: string;
  label: string;
}

export interface ControlBarProps {
  message?: React.ReactNode;
  controls: ControlHelp[];
}

export const ControlBar: React.FC<ControlBarProps> = ({
  message,
  controls,
}) => {
  const { theme } = useTheme();
  const activeKey = useAppStore((state) => state.activeKey);

  return (
    <Box
      marginTop={1}
      borderStyle="double"
      borderColor={theme.border}
      paddingX={1}
    >
      <Box flexGrow={1}>{message}</Box>
      <Box flexWrap="wrap">
        {controls.map((ctrl, idx) => {
          const isActive =
            activeKey === ctrl.key ||
            (ctrl.key === "↑/↓" &&
              (activeKey === "up" || activeKey === "down"));
          return (
            <React.Fragment key={ctrl.key}>
              {idx > 0 && <Text>, </Text>}
              <Color styles={isActive ? theme.keyActive : theme.keyInactive}>
                <Text>{ctrl.key}</Text>
              </Color>
              <Color
                styles={isActive ? theme.labelActive : theme.labelInactive}
              >
                <Text> ({ctrl.label})</Text>
              </Color>
            </React.Fragment>
          );
        })}
      </Box>
    </Box>
  );
};
