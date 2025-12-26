import { Box, Text, useApp, useInput } from "ink";
import Color from "ink-color-pipe";
import type React from "react";
import { useState } from "react";
import type { Config, ConfigManager } from "@/core/config";
import { ControlBar } from "@/tui/components/ControlBar";
import { useTheme } from "@/tui/hooks/useTheme";
import { useToast } from "@/tui/hooks/useToast";
import { useAppStore } from "@/tui/store/useAppStore";

interface MainMenuProps {
  config: Config;
  configManager: ConfigManager;
  initialSelection?: string | null;
  onSelect: (option: string) => void;
}
const OPTIONS = [
  { id: "update", label: "Update All" },
  { id: "install", label: "Install Addon" },
  { id: "manage", label: "Manage Addons" },
  { id: "config", label: "Settings" },
] as const;

export const MainMenu: React.FC<MainMenuProps> = ({
  config,
  configManager,
  initialSelection,
  onSelect,
}) => {
  const { exit } = useApp();
  const { theme } = useTheme();
  const flashKey = useAppStore((state) => state.flashKey);
  const [selectedIndex, setSelectedIndex] = useState(() => {
    const targetId = initialSelection || config.defaultMenuOption;
    const idx = OPTIONS.findIndex((opt) => opt.id === targetId);
    return idx !== -1 ? idx : 0;
  });

  const [defaultOption, setDefaultOption] = useState(config.defaultMenuOption);
  const { toast, showToast } = useToast();

  useInput((input, key) => {
    if (key.upArrow || input === "k") {
      flashKey("↑/↓");
      setSelectedIndex((prev) => (prev > 0 ? prev - 1 : OPTIONS.length - 1));
    } else if (key.downArrow || input === "j") {
      flashKey("↑/↓");
      setSelectedIndex((prev) => (prev < OPTIONS.length - 1 ? prev + 1 : 0));
    } else if (key.return) {
      flashKey("enter");
      const selected = OPTIONS[selectedIndex];
      if (selected) {
        onSelect(selected.id);
      }
    } else if (input === " " || key.rightArrow || input === "l") {
      flashKey("space");
      // Set Default
      const selected = OPTIONS[selectedIndex];
      if (selected) {
        if (selected.id === "config") {
          showToast("Why would you even want that?", 2000);
        } else {
          const newDefault = selected.id as "update" | "manage" | "config";
          configManager.set("defaultMenuOption", newDefault);
          setDefaultOption(newDefault);
          showToast("Default Updated", 2000);
        }
      }
    } else if (input === "q" || key.escape) {
      flashKey("q");
      exit();
    }
  });

  return (
    <Box flexDirection="column" gap={1}>
      {OPTIONS.map((opt, index) => {
        const isSelected = index === selectedIndex;
        const isDefault = opt.id === defaultOption;

        const getStyle = () => {
          if (isSelected) return theme.heading;
          if (isDefault) return theme.warning;
          return theme.labelInactive;
        };

        return (
          <Box key={opt.id}>
            <Color styles={isSelected ? theme.heading : theme.muted}>
              <Text>{isSelected ? "> " : "  "}</Text>
            </Color>
            <Color styles={getStyle()}>
              <Text bold={isSelected || isDefault}>{opt.label}</Text>
            </Color>
          </Box>
        );
      })}
      <ControlBar
        message={
          toast?.message ? (
            <Color styles={theme.warning}>
              <Text>{toast.message}</Text>
            </Color>
          ) : undefined
        }
        controls={[
          { key: "↑/↓", label: "nav" },
          { key: "enter", label: "select" },
          {
            key: "space",
            label: (
              <>
                set{" "}
                <Color styles={theme.warning}>
                  <Text bold>default</Text>
                </Color>
              </>
            ),
          },
          { key: "q", label: "quit" },
        ]}
      />
    </Box>
  );
};
