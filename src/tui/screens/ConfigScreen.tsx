import { Box, Text, useInput } from "ink";
import Color from "ink-color-pipe";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useState } from "react";
import type { ConfigManager } from "@/core/config";
import { logger } from "@/core/logger";
import { ControlBar } from "@/tui/components/ControlBar";
import { ScreenTitle } from "@/tui/components/ScreenTitle";
import { useTheme } from "@/tui/hooks/useTheme";
import { useToast } from "@/tui/hooks/useToast";
import { useAppStore } from "@/tui/store/useAppStore";
import type { Theme } from "@/tui/theme";

interface ScreenProps {
  configManager: ConfigManager;
  onBack: () => void;
}

type Field =
  | "destDir"
  | "maxConcurrent"
  | "checkInterval"
  | "autoCheck"
  | "backupWTF"
  | "backupRetention"
  | "nerdFonts"
  | "themeMode"
  | "debug";

const SectionHeader: React.FC<{
  title: string;
  first?: boolean;
  theme: Theme;
}> = ({ title, first, theme }) => (
  <Box marginTop={first ? 0 : 1} marginBottom={0}>
    <Color styles={theme.statusWarning}>
      <Text bold underline>
        {title}
      </Text>
    </Color>
  </Box>
);

export const ConfigScreen: React.FC<ScreenProps> = ({
  configManager,
  onBack,
}) => {
  const flashKey = useAppStore((state) => state.flashKey);
  const devMode = useAppStore((state) => state.devMode);
  const { theme, themeMode, setTheme } = useTheme();

  const [maxConcurrent, setMaxConcurrent] = useState(3);
  const [destDir, setDestDir] = useState("");
  const [nerdFonts, setNerdFonts] = useState(true);
  const [checkInterval, setCheckInterval] = useState(60);
  const [autoCheckEnabled, setAutoCheckEnabled] = useState(false);
  const [autoCheckInterval, setAutoCheckInterval] = useState(60); // minutes
  const [backupWTF, setBackupWTF] = useState(true);
  const [backupRetention, setBackupRetention] = useState(5);
  const [debug, setDebug] = useState(false);
  const [commandHelp, setCommandHelp] = useState<string | null>(null);

  const [activeField, setActiveField] = useState<Field>("destDir");
  const [isEditingDestDir, setIsEditingDestDir] = useState(false);
  const { toast, showToast } = useToast();

  const getNextInterval = (current: number) => {
    if (current < 60) return Math.min(60, current + 10);
    if (current < 900) return Math.min(900, current + 60);
    return Math.min(3600, current + 300);
  };

  const getPrevInterval = (current: number) => {
    if (current <= 60) return Math.max(0, current - 10);
    if (current <= 900) return Math.max(60, current - 60);
    return Math.max(900, current - 300);
  };

  const formatInterval = (seconds: number) => {
    if (seconds < 60) return `${seconds}s`;
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    if (secs === 0) return `${mins}m`;
    return `${mins}m ${secs}s`;
  };

  // Auto-check interval: 30min steps from 30min to 4hr (dev mode allows lower)
  const autoCheckIntervals = devMode
    ? [1, 5, 15, 30, 60, 90, 120, 150, 180, 210, 240]
    : [30, 60, 90, 120, 150, 180, 210, 240];

  const minAutoInterval = devMode ? 1 : 30;

  const formatAutoInterval = (mins: number) => {
    if (mins < 60) return `${mins}m`;
    const hrs = Math.floor(mins / 60);
    const rem = mins % 60;
    return rem === 0 ? `${hrs}hr` : `${hrs}hr ${rem}m`;
  };

  const getNextAutoInterval = (current: number): number => {
    const idx = autoCheckIntervals.indexOf(current);
    if (idx === -1) return autoCheckIntervals[0] ?? 60;
    return (
      autoCheckIntervals[Math.min(autoCheckIntervals.length - 1, idx + 1)] ?? 60
    );
  };

  const getPrevAutoInterval = (current: number): number => {
    const idx = autoCheckIntervals.indexOf(current);
    if (idx === -1) return autoCheckIntervals[0] ?? minAutoInterval;
    return autoCheckIntervals[Math.max(0, idx - 1)] ?? minAutoInterval;
  };

  useEffect(() => {
    const cfg = configManager.get();
    setMaxConcurrent(cfg.maxConcurrent);
    setDestDir(cfg.destDir === "NOT_CONFIGURED" ? "" : cfg.destDir);
    setNerdFonts(cfg.nerdFonts);
    setCheckInterval(cfg.checkInterval / 1000);
    setAutoCheckEnabled(cfg.autoCheckEnabled);
    setAutoCheckInterval(cfg.autoCheckInterval / 1000 / 60); // ms -> minutes
    setBackupWTF(cfg.backupWTF);
    setBackupRetention(cfg.backupRetention);
    setDebug(cfg.debug);
  }, [configManager]);

  useInput((input, key) => {
    // If editing destDir, trap all input except Enter/Escape
    if (isEditingDestDir) {
      if (key.return) {
        flashKey("enter");
        configManager.set("destDir", destDir);
        setIsEditingDestDir(false);
        showToast("Saved!", 1000);
      } else if (key.escape) {
        flashKey("esc");
        // Revert changes
        const cfg = configManager.get();
        setDestDir(cfg.destDir === "NOT_CONFIGURED" ? "" : cfg.destDir);
        setIsEditingDestDir(false);
      }
      return;
    }

    const fields: Field[] = [
      "destDir",
      "maxConcurrent",
      "checkInterval",
      "autoCheck",
      "backupWTF",
      "backupRetention",
      "nerdFonts",
      "themeMode",
      "debug",
    ];

    if (key.upArrow || input === "k") {
      flashKey("↑/↓");
      const idx = fields.indexOf(activeField);
      const prev = fields[Math.max(0, idx - 1)];
      if (prev) setActiveField(prev);
      return;
    }
    if (key.downArrow || key.tab || input === "j") {
      flashKey("↑/↓");
      const idx = fields.indexOf(activeField);
      const next = fields[Math.min(fields.length - 1, idx + 1)];
      if (next) setActiveField(next);
      return;
    }

    if (key.escape) {
      flashKey("esc");
      onBack();
      return;
    }

    if (activeField === "maxConcurrent") {
      if (key.leftArrow || input === "h") {
        flashKey("←/→");
        const newVal = Math.max(1, maxConcurrent - 1);
        setMaxConcurrent(newVal);
        configManager.set("maxConcurrent", newVal);
        showToast("Saved!", 1000);
      }
      if (key.rightArrow || input === "l") {
        flashKey("←/→");
        const newVal = Math.min(10, maxConcurrent + 1);
        setMaxConcurrent(newVal);
        configManager.set("maxConcurrent", newVal);
        showToast("Saved!", 1000);
      }
    }

    if (activeField === "destDir" && (key.return || input === " ")) {
      flashKey("enter");
      setIsEditingDestDir(true);
    }

    if (activeField === "nerdFonts") {
      if (
        key.leftArrow ||
        key.rightArrow ||
        input === "h" ||
        input === "l" ||
        input === " "
      ) {
        flashKey(input === " " ? "space" : "←/→");
        setNerdFonts(!nerdFonts);
        configManager.set("nerdFonts", !nerdFonts);
        showToast("Saved!", 1000);
      }
    }

    if (activeField === "themeMode") {
      if (
        key.leftArrow ||
        key.rightArrow ||
        input === "h" ||
        input === "l" ||
        input === " "
      ) {
        flashKey(input === " " ? "space" : "←/→");
        const nextTheme = themeMode === "dark" ? "light" : "dark";
        setTheme(nextTheme);
        configManager.set("theme", nextTheme);
        showToast("Theme Updated!", 1000);
      }
    }
    if (activeField === "checkInterval") {
      if (key.leftArrow || input === "h") {
        flashKey("←/→");
        const newVal = getPrevInterval(checkInterval);
        setCheckInterval(newVal);
        configManager.set("checkInterval", newVal * 1000);
        showToast("Saved!", 1000);
      }
      if (key.rightArrow || input === "l") {
        flashKey("←/→");
        const newVal = getNextInterval(checkInterval);
        setCheckInterval(newVal);
        configManager.set("checkInterval", newVal * 1000);
        showToast("Saved!", 1000);
      }
    }

    if (activeField === "autoCheck") {
      // Space/Enter toggles enabled
      if (input === " " || key.return) {
        flashKey(input === " " ? "space" : "enter");
        const newEnabled = !autoCheckEnabled;
        setAutoCheckEnabled(newEnabled);
        configManager.set("autoCheckEnabled", newEnabled);
        showToast(newEnabled ? "Enabled" : "Disabled", 1000);
      }
      // Left: decrease interval or disable at minimum
      if (key.leftArrow || input === "h") {
        flashKey("←/→");
        if (autoCheckEnabled) {
          // At minimum, disable instead of staying at min
          if (autoCheckInterval <= minAutoInterval) {
            setAutoCheckEnabled(false);
            configManager.set("autoCheckEnabled", false);
            showToast("Disabled", 1000);
          } else {
            const newVal = getPrevAutoInterval(autoCheckInterval);
            setAutoCheckInterval(newVal);
            configManager.set("autoCheckInterval", newVal * 60 * 1000);
            showToast("Saved!", 1000);
          }
        }
      }
      // Right: enable at minimum or increase interval
      if (key.rightArrow || input === "l") {
        flashKey("←/→");
        if (!autoCheckEnabled) {
          // Enable and set to minimum interval
          setAutoCheckEnabled(true);
          setAutoCheckInterval(minAutoInterval);
          configManager.set("autoCheckEnabled", true);
          configManager.set("autoCheckInterval", minAutoInterval * 60 * 1000);
          showToast("Enabled", 1000);
        } else {
          const newVal = getNextAutoInterval(autoCheckInterval);
          setAutoCheckInterval(newVal);
          configManager.set("autoCheckInterval", newVal * 60 * 1000);
          showToast("Saved!", 1000);
        }
      }
    }

    if (activeField === "backupWTF") {
      if (
        key.leftArrow ||
        key.rightArrow ||
        input === "h" ||
        input === "l" ||
        input === " "
      ) {
        flashKey(input === " " ? "space" : "←/→");
        setBackupWTF(!backupWTF);
        configManager.set("backupWTF", !backupWTF);
        showToast("Saved!", 1000);
      }
    }

    if (activeField === "backupRetention") {
      if (key.leftArrow || input === "h") {
        flashKey("←/→");
        const newVal = Math.max(1, backupRetention - 1);
        setBackupRetention(newVal);
        configManager.set("backupRetention", newVal);
        showToast("Saved!", 1000);
      }
      if (key.rightArrow || input === "l") {
        flashKey("←/→");
        const newVal = Math.min(20, backupRetention + 1);
        setBackupRetention(newVal);
        configManager.set("backupRetention", newVal);
        showToast("Saved!", 1000);
      }
    }

    if (activeField === "debug") {
      if (
        key.leftArrow ||
        key.rightArrow ||
        input === "h" ||
        input === "l" ||
        input === " "
      ) {
        flashKey(input === " " ? "space" : "←/→");
        setDebug(!debug);
        configManager.set("debug", !debug);
        showToast("Saved!", 1000);
      }
    }
  });

  const ConfigOption: React.FC<{
    label: string;
    isActive: boolean;
    helpText?: string;
    children: React.ReactNode;
  }> = ({ label, isActive, helpText, children }) => {
    useEffect(() => {
      if (isActive) {
        setCommandHelp(
          helpText || "Use Left/Right arrows to change. Up/Down to switch.",
        );
      }
    }, [isActive, helpText]);

    return (
      <Box paddingLeft={1}>
        <Color styles={isActive ? theme.selection : undefined}>
          <Text>{isActive ? "› " : "  "}</Text>
        </Color>
        <Box width={28}>
          <Color styles={isActive ? theme.highlight : theme.labelInactive}>
            <Text bold={isActive}>{label}</Text>
          </Color>
        </Box>
        <Box flexGrow={1}>{children}</Box>
      </Box>
    );
  };

  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <ScreenTitle title="Settings" />
      </Box>

      <Box flexDirection="column" gap={0}>
        {/* General */}
        <SectionHeader title="General" first theme={theme} />
        <ConfigOption
          label="WoW AddOns Directory"
          isActive={activeField === "destDir"}
          helpText={
            isEditingDestDir
              ? "Press Enter to save, Esc to cancel."
              : "Press Enter or Space to edit path."
          }
        >
          {isEditingDestDir ? (
            <Box>
              <Color styles={theme.statusChecking}>
                <Text bold>[EDITING]</Text>
              </Color>
              <Box marginLeft={1}>
                <TextInput
                  value={destDir}
                  onChange={setDestDir}
                  onSubmit={() => {}}
                />
              </Box>
            </Box>
          ) : (
            <Color styles={destDir ? theme.labelInactive : theme.statusIdle}>
              <Text bold>{destDir || "Not Configured"}</Text>
            </Color>
          )}
        </ConfigOption>

        {/* Updates */}
        <SectionHeader title="Updates" theme={theme} />
        <ConfigOption
          label="Max Concurrent Downloads"
          isActive={activeField === "maxConcurrent"}
        >
          <Color styles={theme.statusChecking}>
            <Text bold>
              {"◂"} {maxConcurrent} {"▸"}
            </Text>
          </Color>
        </ConfigOption>
        <ConfigOption
          label="Check Interval"
          isActive={activeField === "checkInterval"}
        >
          <Color styles={theme.statusChecking}>
            <Text bold>
              {"◂"} {formatInterval(checkInterval)} {"▸"}
            </Text>
          </Color>
        </ConfigOption>
        <ConfigOption
          label="Auto-check in Background"
          isActive={activeField === "autoCheck"}
          helpText={
            autoCheckEnabled
              ? "Press Space to disable. Left/Right to adjust interval."
              : "Press Space to enable background update checking."
          }
        >
          {autoCheckEnabled ? (
            <Color styles={theme.statusChecking}>
              <Text bold>
                {"◂"} {formatAutoInterval(autoCheckInterval)} {"▸"}
              </Text>
            </Color>
          ) : (
            <Color styles={theme.statusError}>
              <Text bold>Disabled</Text>
            </Color>
          )}
        </ConfigOption>

        {/* Backup */}
        <SectionHeader title="Backup" theme={theme} />
        <ConfigOption
          label="Auto-backup WTF Folder"
          isActive={activeField === "backupWTF"}
          helpText="Backup only occurs when running 'Update All'."
        >
          <Color styles={backupWTF ? theme.statusSuccess : theme.statusError}>
            <Text bold>{backupWTF ? "Enabled" : "Disabled"}</Text>
          </Color>
        </ConfigOption>
        <ConfigOption
          label="Backup Retention"
          isActive={activeField === "backupRetention"}
        >
          <Color styles={theme.statusChecking}>
            <Text bold>
              {"◂"} {backupRetention} backups {"▸"}
            </Text>
          </Color>
        </ConfigOption>

        {/* Appearance */}
        <SectionHeader title="Appearance" theme={theme} />
        <ConfigOption
          label="Nerd Fonts (Icons)"
          isActive={activeField === "nerdFonts"}
        >
          <Color styles={nerdFonts ? theme.statusSuccess : theme.statusError}>
            <Text bold>{nerdFonts ? "Enabled" : "Disabled"}</Text>
          </Color>
        </ConfigOption>
        <ConfigOption label="Theme" isActive={activeField === "themeMode"}>
          <Color
            styles={
              themeMode === "dark" ? theme.repoTukui : theme.statusWorking
            }
          >
            <Text bold>{themeMode === "dark" ? "Dark" : "Light"}</Text>
          </Color>
        </ConfigOption>

        {/* Advanced */}
        <SectionHeader title="Advanced" theme={theme} />
        <ConfigOption label="Debug Logging" isActive={activeField === "debug"}>
          <Box>
            <Color styles={debug ? theme.statusSuccess : theme.statusIdle}>
              <Text bold>{debug ? "Enabled" : "Disabled"}</Text>
            </Color>
            {debug && (
              <Box marginLeft={2}>
                <Color styles={theme.statusIdle}>
                  <Text>({logger.getLogPath()})</Text>
                </Color>
              </Box>
            )}
          </Box>
        </ConfigOption>
      </Box>

      <ControlBar
        message={
          toast?.message ? (
            <Color styles={theme.statusSuccess}>
              <Text>{toast.message}</Text>
            </Color>
          ) : commandHelp ? (
            <Text>{commandHelp}</Text>
          ) : undefined
        }
        controls={[
          { key: "↑/↓", label: "nav" },
          { key: "←/→", label: "modify" },
          { key: "space", label: "toggle/edit" },
          { key: "enter", label: "edit" },
          { key: "esc", label: "back" },
        ]}
      />
    </Box>
  );
};
