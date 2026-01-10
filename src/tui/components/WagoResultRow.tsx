import { Box, Text } from "ink";
import Color from "ink-color-pipe";
import type React from "react";
import type { WagoAddonSummary } from "@/core/wago";
import { useTheme } from "@/tui/hooks/useTheme";

interface WagoResultRowProps {
  addon: WagoAddonSummary;
  isSelected: boolean;
  isInstalled: boolean;
}

function formatDownloads(count: number | undefined): string {
  if (!count) return "-";
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${Math.floor(count / 1_000)}K`;
  return count.toString();
}

function truncate(str: string, max: number): string {
  if (!str) return "-";
  return str.length > max ? `${str.substring(0, max - 1)}…` : str;
}

export const WagoResultRow: React.FC<WagoResultRowProps> = ({
  addon,
  isSelected,
  isInstalled,
}) => {
  const { theme } = useTheme();

  const author = addon.owner ?? addon.authors?.[0] ?? "-";
  const downloads = formatDownloads(addon.download_count);

  const nameColor = isSelected
    ? theme.highlight
    : isInstalled
      ? theme.success
      : undefined;

  return (
    <Box>
      <Box width={2} flexShrink={0}>
        <Color styles={isSelected ? theme.selection : undefined}>
          <Text>{isSelected ? ">" : " "}</Text>
        </Color>
      </Box>

      <Box width={30} flexShrink={0}>
        <Color styles={nameColor}>
          <Text bold={isSelected} wrap="truncate-end">
            {isInstalled ? "✔ " : ""}
            {addon.display_name}
          </Text>
        </Color>
      </Box>

      <Box width={14} flexShrink={0}>
        <Color styles={theme.muted}>
          <Text wrap="truncate-end">{truncate(author, 12)}</Text>
        </Color>
      </Box>

      <Box width={8} flexShrink={0} justifyContent="flex-end">
        <Color styles={theme.version}>
          <Text>{downloads}</Text>
        </Color>
      </Box>

      <Box flexGrow={1} marginLeft={2} minWidth={0}>
        <Color styles={theme.muted}>
          <Text wrap="truncate-end">{addon.summary}</Text>
        </Color>
      </Box>
    </Box>
  );
};
