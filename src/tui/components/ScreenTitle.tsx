import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import type React from "react";
import { THEME } from "../theme";

interface ScreenTitleProps {
  title: string;
  children?: React.ReactNode;
}

export const ScreenTitle: React.FC<ScreenTitleProps> = ({
  title,
  children,
}) => (
  <Box flexDirection="row" gap={2}>
    <Color styles={THEME.heading}>
      <Text>{title}</Text>
    </Color>
    {children}
  </Box>
);
