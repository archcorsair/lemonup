import { Box, Text, useInput } from "ink";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useState } from "react";
import type { ConfigManager } from "@/core/config";
import { ControlBar } from "@/tui/components/ControlBar";
import { ScreenTitle } from "@/tui/components/ScreenTitle";
import { useToast } from "@/tui/hooks/useToast";
import { useAppStore } from "@/tui/store/useAppStore";

interface ScreenProps {
  configManager: ConfigManager;
  onBack: () => void;
}

type Field =
  | "destDir"
  | "maxConcurrent"
  | "checkInterval"
  | "backupWTF"
  | "backupRetention"
  | "nerdFonts"
  | "debug";

const SectionHeader: React.FC<{ title: string; first?: boolean }> = ({
  title,
  first,
}) => (
  <Box marginTop={first ? 0 : 1} marginBottom={0}>
    <Text color="yellow" bold underline>
      {title}
    </Text>
  </Box>
);

export const ConfigScreen: React.FC<ScreenProps> = ({
  configManager,
  onBack,
}) => {
  const flashKey = useAppStore((state) => state.flashKey);
  const [maxConcurrent, setMaxConcurrent] = useState(3);
  const [destDir, setDestDir] = useState("");
  const [nerdFonts, setNerdFonts] = useState(true);
  const [checkInterval, setCheckInterval] = useState(60);
  const [backupWTF, setBackupWTF] = useState(true);
  const [backupRetention, setBackupRetention] = useState(5);
  const [debug, setDebug] = useState(false);
  const [commandHelp, setCommandHelp] = useState<string | null>(null);

  const [activeField, setActiveField] = useState<Field>("destDir");
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

  useEffect(() => {
    const cfg = configManager.get();
    setMaxConcurrent(cfg.maxConcurrent);
    setDestDir(cfg.destDir === "NOT_CONFIGURED" ? "" : cfg.destDir);
    setNerdFonts(cfg.nerdFonts);
    setCheckInterval(cfg.checkInterval / 1000);
    setBackupWTF(cfg.backupWTF);
    setBackupRetention(cfg.backupRetention);
    setDebug(cfg.debug);
  }, [configManager]);

  useInput((input, key) => {
    const fields: Field[] = [
      "destDir",
      "maxConcurrent",
      "checkInterval",
      "backupWTF",
      "backupRetention",
      "nerdFonts",
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

    if (activeField === "destDir" && key.return) {
      flashKey("enter");
      configManager.set("destDir", destDir);
      showToast("Saved!", 1000);
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
        <Text color={isActive ? "cyan" : "whiteBright"} dimColor={!isActive}>
          {isActive ? "› " : "  "}
        </Text>
        <Box width={28}>
          <Text
            color={isActive ? "white" : "whiteBright"}
            bold={isActive}
            dimColor={!isActive}
          >
            {label}
          </Text>
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
        <SectionHeader title="General" first />
        <ConfigOption
          label="WoW AddOns Directory"
          isActive={activeField === "destDir"}
          helpText="Type path and press Enter to save."
        >
          {activeField === "destDir" ? (
            <TextInput
              value={destDir}
              onChange={setDestDir}
              onSubmit={(val) => {
                configManager.set("destDir", val);
                showToast("Saved!", 1000);
              }}
            />
          ) : (
            <Text color={destDir ? "white" : "gray"}>
              {destDir || "Not Configured"}
            </Text>
          )}
        </ConfigOption>

        {/* Updates */}
        <SectionHeader title="Updates" />
        <ConfigOption
          label="Max Concurrent Downloads"
          isActive={activeField === "maxConcurrent"}
        >
          <Text color="yellow" bold>
            {"◂"} {maxConcurrent} {"▸"}
          </Text>
        </ConfigOption>
        <ConfigOption
          label="Check Interval"
          isActive={activeField === "checkInterval"}
        >
          <Text color="yellow">
            {"◂"} {formatInterval(checkInterval)} {"▸"}
          </Text>
        </ConfigOption>

        {/* Backup */}
        <SectionHeader title="Backup" />
        <ConfigOption
          label="Auto-backup WTF Folder"
          isActive={activeField === "backupWTF"}
        >
          <Text color={backupWTF ? "green" : "red"}>
            {backupWTF ? "Enabled" : "Disabled"}
          </Text>
        </ConfigOption>
        <ConfigOption
          label="Backup Retention"
          isActive={activeField === "backupRetention"}
        >
          <Text color="yellow" bold>
            {"◂"} {backupRetention} backups {"▸"}
          </Text>
        </ConfigOption>

        {/* Appearance */}
        <SectionHeader title="Appearance" />
        <ConfigOption
          label="Nerd Fonts (Icons)"
          isActive={activeField === "nerdFonts"}
        >
          <Text color={nerdFonts ? "green" : "red"}>
            {nerdFonts ? "Enabled" : "Disabled"}
          </Text>
        </ConfigOption>

        {/* Advanced */}
        <SectionHeader title="Advanced" />
        <ConfigOption label="Debug Logging" isActive={activeField === "debug"}>
          <Text color={debug ? "green" : "gray"}>
            {debug ? "Enabled" : "Disabled"}
          </Text>
        </ConfigOption>
      </Box>

      <ControlBar
        message={
          toast?.message ? (
            <Text color="green">{toast.message}</Text>
          ) : commandHelp ? (
            <Text>{commandHelp}</Text>
          ) : undefined
        }
        controls={[
          { key: "↑/↓", label: "nav" },
          { key: "←/→", label: "modify" },
          { key: "space", label: "toggle" },
          { key: "enter", label: "save text" },
          { key: "esc", label: "back" },
        ]}
      />
    </Box>
  );
};
