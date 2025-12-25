import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import React from "react";
import { useAppStore } from "@/tui/store/useAppStore";
import { THEME } from "../theme";

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
  const activeKey = useAppStore((state) => state.activeKey);

  return (
    <Box
      marginTop={1}
      borderStyle="double"
      borderColor={THEME.border}
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
              <Color styles={isActive ? THEME.keyActive : THEME.keyInactive}>
                <Text>{ctrl.key}</Text>
              </Color>
              <Color
                styles={isActive ? THEME.labelActive : THEME.labelInactive}
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
