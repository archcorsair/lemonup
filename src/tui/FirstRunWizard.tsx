import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Box, Text, useApp, useInput } from "ink";
import Color from "ink-color-pipe";
import Spinner from "ink-spinner";
import TextInput from "ink-text-input";
import type React from "react";
import { useRef, useState } from "react";
import type { ConfigManager } from "@/core/config";
import { getDefaultWoWPath, searchForWoW } from "@/core/paths";
import { ControlBar } from "./components/ControlBar";
import { useTheme } from "./hooks/useTheme";
import { useAppStore } from "./store/useAppStore";

interface FirstRunWizardProps {
  configManager: ConfigManager;
  onComplete: () => void;
}

interface WizardState {
  theme: "dark" | "light";
  destDir: string;
  destDirMode: "auto" | "manual";
  installElvUI: boolean;
  installTukui: boolean;
  maxConcurrent: number;
  checkInterval: number; // seconds
  autoCheckEnabled: boolean;
  autoCheckInterval: number; // minutes
  backupWTF: boolean;
  backupRetention: number;
}

const TOTAL_STEPS = 5;
const STEP_NAMES = ["Theme", "Directory", "Addons", "Settings", "Review"];

const expandPath = (p: string): string => {
  if (p.startsWith("~/") || p === "~") {
    return path.join(os.homedir(), p.slice(1));
  }
  return p;
};

// Progress bar component
const WizardProgress: React.FC<{
  currentStep: number;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ currentStep, theme }) => {
  const segmentWidth = 12;
  const halfSegment = Math.floor(segmentWidth / 2);

  // Build the progress bar with markers embedded
  const renderProgressBar = () => {
    const elements: React.ReactNode[] = [];

    for (let i = 0; i < TOTAL_STEPS; i++) {
      const isCompleted = i + 1 < currentStep;
      const isCurrent = i + 1 === currentStep;
      const segmentColor =
        i < currentStep ? theme.progressCompleted : theme.progressPending;

      // Adjust segment length if marker is wider (checked state)
      const segmentLen = isCompleted ? halfSegment - 1 : halfSegment;

      // Add leading half-segment (before marker)
      elements.push(
        <Color key={`pre-${i}`} styles={segmentColor}>
          <Text>{"━".repeat(segmentLen)}</Text>
        </Color>,
      );

      // Add marker
      const marker = isCompleted ? " ✓ " : isCurrent ? "●" : "○";
      const markerColor = isCompleted
        ? theme.progressCompleted
        : isCurrent
          ? theme.progressCurrent
          : theme.progressPending;
      elements.push(
        <Color key={`mark-${i}`} styles={markerColor}>
          <Text bold={isCurrent}>{marker}</Text>
        </Color>,
      );

      // Add trailing half-segment (after marker)
      const trailingColor =
        i + 1 < currentStep ? theme.progressCompleted : theme.progressPending;
      elements.push(
        <Color key={`post-${i}`} styles={trailingColor}>
          <Text>{"━".repeat(segmentLen)}</Text>
        </Color>,
      );
    }

    return elements;
  };

  return (
    <Box flexDirection="column" marginBottom={1} paddingLeft={1}>
      {/* Progress bar line with markers */}
      <Box>{renderProgressBar()}</Box>

      {/* Labels row - evenly spaced under markers */}
      <Box>
        {STEP_NAMES.map((name, idx) => {
          const stepColor =
            idx + 1 === currentStep
              ? theme.progressCurrent
              : idx + 1 < currentStep
                ? theme.progressCompleted
                : theme.progressPending;
          return (
            <Box key={name} width={segmentWidth + 1} justifyContent="center">
              <Color styles={stepColor}>
                <Text bold={idx + 1 === currentStep}>{name}</Text>
              </Color>
            </Box>
          );
        })}
      </Box>
    </Box>
  );
};

// Step 1: Theme Selection
const ThemeStep: React.FC<{
  value: "dark" | "light";
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ value, theme }) => {
  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Choose your theme:</Text>
      </Color>
      <Box marginTop={1}>
        <Color styles={theme.muted}>
          <Text>◀ </Text>
        </Color>
        <Color styles={theme.brand}>
          <Text bold> [ {value === "dark" ? "Dark" : "Light"} ] </Text>
        </Color>
        <Color styles={theme.muted}>
          <Text> ▶</Text>
        </Color>
      </Box>
      <Box marginTop={1}>
        <Color styles={theme.muted}>
          <Text>
            The interface will update as you toggle. Use ←/→ to switch.
          </Text>
        </Color>
      </Box>
    </Box>
  );
};

// Step 2: Directory Selection
const DirectoryStep: React.FC<{
  mode: "auto" | "manual";
  destDir: string;
  onDirChange: (d: string) => void;
  isEditing: boolean;
  onEditToggle: (editing: boolean) => void;
  pathValid: boolean | null;
  theme: ReturnType<typeof useTheme>["theme"];
  isScanning: boolean;
  scanError: string | null;
}> = ({
  mode,
  destDir,
  onDirChange,
  isEditing,
  onEditToggle,
  pathValid,
  theme,
  isScanning,
  scanError,
}) => {
  const detectedPath = getDefaultWoWPath();
  const isDetected = detectedPath !== "NOT_CONFIGURED";

  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Where is your WoW AddOns folder?</Text>
      </Color>

      <Box flexDirection="column" marginTop={1}>
        <Box>
          <Color styles={mode === "auto" ? theme.selection : theme.muted}>
            <Text>{mode === "auto" ? "› " : "  "}</Text>
          </Color>
          <Color
            styles={mode === "auto" ? theme.highlight : theme.labelInactive}
          >
            <Text bold={mode === "auto"}>Auto-detect</Text>
          </Color>
        </Box>
        <Box>
          <Color styles={mode === "manual" ? theme.selection : theme.muted}>
            <Text>{mode === "manual" ? "› " : "  "}</Text>
          </Color>
          <Color
            styles={mode === "manual" ? theme.highlight : theme.labelInactive}
          >
            <Text bold={mode === "manual"}>Manual input</Text>
          </Color>
        </Box>
      </Box>

      <Box marginTop={1} flexDirection="column">
        {mode === "auto" ? (
          isDetected ? (
            <Box>
              <Color styles={theme.success}>
                <Text>Detected: </Text>
              </Color>
              <Color styles={theme.labelInactive}>
                <Text>{detectedPath}</Text>
              </Color>
            </Box>
          ) : (
            <Box flexDirection="column">
              <Color styles={theme.warning}>
                <Text>
                  Could not auto-detect WoW path. Please use manual input.
                </Text>
              </Color>
              <Box marginTop={1}>
                {isScanning ? (
                  <Box>
                    <Color styles={theme.brand}>
                      {/* @ts-expect-error ink-spinner types are not fully compatible with React 19 */}
                      <Spinner type="dots" />
                    </Color>
                    <Text>
                      {" "}
                      Scanning for WoW installation... (Press Esc to cancel)
                    </Text>
                  </Box>
                ) : (
                  <Box flexDirection="column">
                    <Text>
                      or press{" "}
                      <Color styles={theme.keyActive}>
                        <Text bold>S</Text>
                      </Color>{" "}
                      to perform a deep scan.
                    </Text>
                    {scanError && (
                      <Color styles={theme.error}>
                        <Text>Scan failed: {scanError}</Text>
                      </Color>
                    )}
                  </Box>
                )}
              </Box>
            </Box>
          )
        ) : isEditing ? (
          <Box>
            <Color styles={theme.brand}>
              <Text bold>✎ </Text>
            </Color>
            <TextInput
              value={destDir}
              onChange={onDirChange}
              placeholder="Enter path to WoW AddOns folder..."
              onSubmit={() => onEditToggle(false)}
            />
          </Box>
        ) : (
          <Box>
            <Box width={3}>
              {destDir && pathValid !== null && (
                <Color styles={pathValid ? theme.success : theme.error}>
                  <Text bold>{pathValid ? " ✓" : " ✗"}</Text>
                </Color>
              )}
            </Box>
            <Color styles={theme.labelInactive}>
              <Text>Path: {destDir || "(press Enter to edit)"}</Text>
            </Color>
          </Box>
        )}
      </Box>

      <Box marginTop={1}>
        <Color styles={theme.muted}>
          <Text>
            Use ↑/↓ to switch mode.{" "}
            {mode === "manual" && !isEditing && "Space to edit path."}
          </Text>
        </Color>
      </Box>
    </Box>
  );
};

// Step 3: Addon Selection
const AddonsStep: React.FC<{
  installElvUI: boolean;
  installTukui: boolean;
  selectedIndex: number;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ installElvUI, installTukui, selectedIndex, theme }) => {
  const addons = [
    {
      id: "elvui" as const,
      name: "ElvUI",
      desc: "Complete UI replacement",
      checked: installElvUI,
    },
    {
      id: "tukui" as const,
      name: "Tukui",
      desc: "Minimalist UI framework",
      checked: installTukui,
    },
  ];

  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Would you like to install any UI replacements?</Text>
      </Color>

      <Box flexDirection="column" marginTop={1}>
        {addons.map((addon, idx) => (
          <Box key={addon.id}>
            <Color
              styles={idx === selectedIndex ? theme.selection : theme.muted}
            >
              <Text>{idx === selectedIndex ? "› " : "  "}</Text>
            </Color>
            <Color styles={addon.checked ? theme.checked : theme.unchecked}>
              <Text>[{addon.checked ? "x" : " "}] </Text>
            </Color>
            <Color
              styles={
                idx === selectedIndex ? theme.highlight : theme.labelInactive
              }
            >
              <Text bold={idx === selectedIndex}>{addon.name}</Text>
            </Color>
            <Color styles={theme.muted}>
              <Text> - {addon.desc}</Text>
            </Color>
          </Box>
        ))}
      </Box>

      <Box marginTop={1}>
        <Color styles={theme.muted}>
          <Text>
            Use ↑/↓ to navigate, Space to toggle. You can always add more addons
            later.
          </Text>
        </Color>
      </Box>
    </Box>
  );
};

// Step 4: Settings
const SettingsStep: React.FC<{
  state: WizardState;
  selectedIndex: number;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ state, selectedIndex, theme }) => {
  const formatInterval = (seconds: number) => {
    if (seconds < 60) return `${seconds}s`;
    const mins = Math.floor(seconds / 60);
    return `${mins}m`;
  };

  const formatAutoInterval = (mins: number) => {
    if (mins < 60) return `${mins}m`;
    const hrs = Math.floor(mins / 60);
    const rem = mins % 60;
    return rem === 0 ? `${hrs}hr` : `${hrs}hr ${rem}m`;
  };

  const settings = [
    {
      label: "Max Concurrent Downloads",
      value: `◀ ${state.maxConcurrent} ▶`,
    },
    {
      label: "Check Interval",
      value: `◀ ${formatInterval(state.checkInterval)} ▶`,
    },
    {
      label: "Auto-check in Background",
      value: state.autoCheckEnabled
        ? `Enabled (${formatAutoInterval(state.autoCheckInterval)})`
        : "Disabled",
    },
    {
      label: "Auto-backup WTF Folder",
      value: state.backupWTF ? "Enabled" : "Disabled",
    },
    {
      label: "Backup Retention",
      value: `◀ ${state.backupRetention} backups ▶`,
    },
  ];

  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Configure default settings:</Text>
      </Color>

      <Box flexDirection="column" marginTop={1}>
        {settings.map((setting, idx) => (
          <Box key={setting.label}>
            <Color
              styles={idx === selectedIndex ? theme.selection : theme.muted}
            >
              <Text>{idx === selectedIndex ? "› " : "  "}</Text>
            </Color>
            <Box width={28}>
              <Color
                styles={
                  idx === selectedIndex ? theme.highlight : theme.labelInactive
                }
              >
                <Text bold={idx === selectedIndex}>{setting.label}</Text>
              </Color>
            </Box>
            <Color styles={theme.statusChecking}>
              <Text bold>{setting.value}</Text>
            </Color>
          </Box>
        ))}
      </Box>

      <Box marginTop={1}>
        <Color styles={theme.muted}>
          <Text>
            Use ↑/↓ to navigate, ←/→ to adjust values, Space to toggle.
          </Text>
        </Color>
      </Box>
    </Box>
  );
};

// Step 5: Review
const ReviewStep: React.FC<{
  state: WizardState;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ state, theme }) => {
  const formatAutoInterval = (mins: number) => {
    if (mins < 60) return `${mins}m`;
    const hrs = Math.floor(mins / 60);
    return `${hrs}hr`;
  };

  const addons = [];
  if (state.installElvUI) addons.push("ElvUI");
  if (state.installTukui) addons.push("Tukui");

  const items = [
    { label: "Theme", value: state.theme === "dark" ? "Dark" : "Light" },
    { label: "AddOns Directory", value: state.destDir || "(not set)" },
    { label: "Install", value: addons.length > 0 ? addons.join(", ") : "None" },
    { label: "Max Downloads", value: `${state.maxConcurrent} concurrent` },
    {
      label: "Auto-check",
      value: state.autoCheckEnabled
        ? `Every ${formatAutoInterval(state.autoCheckInterval)}`
        : "Disabled",
    },
    {
      label: "Backup",
      value: state.backupWTF
        ? `Enabled (keep ${state.backupRetention})`
        : "Disabled",
    },
  ];

  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Review your setup:</Text>
      </Color>

      <Box flexDirection="column" marginTop={1}>
        {items.map((item) => (
          <Box key={item.label}>
            <Box width={20}>
              <Color styles={theme.labelInactive}>
                <Text>{item.label}:</Text>
              </Color>
            </Box>
            <Color styles={theme.highlight}>
              <Text bold>{item.value}</Text>
            </Color>
          </Box>
        ))}
      </Box>

      <Box marginTop={2}>
        <Color styles={theme.success}>
          <Text bold>Press Enter to complete setup.</Text>
        </Color>
      </Box>
    </Box>
  );
};

export const FirstRunWizard: React.FC<FirstRunWizardProps> = ({
  configManager,
  onComplete,
}) => {
  const { exit } = useApp();
  const { theme, setTheme } = useTheme();
  const flashKey = useAppStore((state) => state.flashKey);

  const [step, setStep] = useState(1);
  const [error, setError] = useState<string | null>(null);

  // Deep Scan State
  const [isScanning, setIsScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const scanAbortController = useRef<AbortController | null>(null);

  // Wizard state
  const [wizardState, setWizardState] = useState<WizardState>(() => {
    const detectedPath = getDefaultWoWPath();
    return {
      theme: "dark",
      destDir: detectedPath !== "NOT_CONFIGURED" ? detectedPath : "",
      destDirMode: "auto",
      installElvUI: false,
      installTukui: false,
      maxConcurrent: 3,
      checkInterval: 300, // 5 minutes in seconds
      autoCheckEnabled: true,
      autoCheckInterval: 60, // 1 hour in minutes
      backupWTF: true,
      backupRetention: 5,
    };
  });

  // Step-specific state
  const [dirEditMode, setDirEditMode] = useState(false);
  const [pathValid, setPathValid] = useState<boolean | null>(null);
  const [addonIndex, setAddonIndex] = useState(0);
  const [settingsIndex, setSettingsIndex] = useState(0);

  const updateWizardState = (updates: Partial<WizardState>) => {
    setWizardState((prev) => ({ ...prev, ...updates }));
  };

  const handleDeepScan = async () => {
    setIsScanning(true);
    setScanError(null);
    scanAbortController.current = new AbortController();

    try {
      const result = await searchForWoW(
        os.homedir(),
        scanAbortController.current.signal,
      );
      if (result) {
        updateWizardState({ destDir: result });
        setPathValid(true);
      } else {
        setScanError("No WoW installation found in deep scan.");
      }
    } catch (e: unknown) {
      if (e instanceof Error && e.name === "AbortError") {
        // Ignore abort
      } else {
        setScanError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      setIsScanning(false);
      scanAbortController.current = null;
    }
  };

  const handleComplete = () => {
    try {
      // Create config with wizard values
      configManager.createDefaultConfig();

      // Apply wizard settings
      configManager.set("theme", wizardState.theme);
      configManager.set("destDir", wizardState.destDir || getDefaultWoWPath());
      configManager.set("maxConcurrent", wizardState.maxConcurrent);
      configManager.set("checkInterval", wizardState.checkInterval * 1000);
      configManager.set("autoCheckEnabled", wizardState.autoCheckEnabled);
      configManager.set(
        "autoCheckInterval",
        wizardState.autoCheckInterval * 60 * 1000,
      );
      configManager.set("backupWTF", wizardState.backupWTF);
      configManager.set("backupRetention", wizardState.backupRetention);

      // TODO: If ElvUI/Tukui selected, queue installation after wizard

      onComplete();
    } catch (e) {
      setError(String(e));
    }
  };

  const goNext = () => {
    setError(null);
    if (step < TOTAL_STEPS) {
      setStep(step + 1);
    } else {
      handleComplete();
    }
  };

  const goBack = () => {
    setError(null);
    if (step > 1) {
      setStep(step - 1);
    } else {
      exit();
    }
  };

  useInput((input, key) => {
    // Handle escape/backspace for back navigation
    if (key.escape || key.backspace) {
      if (isScanning && scanAbortController.current) {
        scanAbortController.current.abort();
        // State update handled in catch block or explicitly here?
        // Let's do it here to be responsive
        setIsScanning(false);
        flashKey("esc");
        return;
      }

      // Don't go back if editing directory
      if (step === 2 && dirEditMode) {
        flashKey("esc");
        setError(null);
        setDirEditMode(false);
        return;
      }
      flashKey("esc");
      goBack();
      return;
    }

    // Step-specific input handling
    switch (step) {
      case 1: // Theme
        if (key.leftArrow || key.rightArrow || input === "h" || input === "l") {
          flashKey("←/→");
          const newTheme = wizardState.theme === "dark" ? "light" : "dark";
          updateWizardState({ theme: newTheme });
          setTheme(newTheme);
        }
        if (key.return) {
          flashKey("enter");
          goNext();
        }
        break;

      case 2: // Directory
        if (dirEditMode) {
          if (key.return) {
            flashKey("enter");
            setDirEditMode(false);
            // Validate path when done editing
            if (wizardState.destDir) {
              const cleaned = wizardState.destDir.trim();
              const expanded = expandPath(cleaned);
              if (expanded !== wizardState.destDir) {
                updateWizardState({ destDir: expanded });
              }
              const valid = fs.existsSync(expanded);
              setPathValid(valid);
              if (!valid) {
                setError("Directory not found. Please check the path.");
              } else {
                setError(null);
              }
            }
          }
          return; // TextInput handles other input
        }

        if (
          (input === "s" || input === "S") &&
          wizardState.destDirMode === "auto"
        ) {
          if (!isScanning) {
            flashKey("scan");

            handleDeepScan();

            return;
          }
        }

        if (key.upArrow || key.downArrow || input === "j" || input === "k") {
          flashKey("↑/↓");
          setError(null);
          const newMode =
            wizardState.destDirMode === "auto" ? "manual" : "auto";
          updateWizardState({ destDirMode: newMode });
          if (newMode === "auto") {
            const detected = getDefaultWoWPath();
            if (detected !== "NOT_CONFIGURED") {
              updateWizardState({ destDir: detected });
              setPathValid(fs.existsSync(detected));
            }
          } else {
            setPathValid(null); // Reset validation when switching to manual
          }
        }

        // Space bar to re-edit path
        if (input === " ") {
          flashKey("space");
          setError(null);
          setDirEditMode(true);
          setPathValid(null); // Reset validation
          return;
        }

        if (key.return) {
          flashKey("enter");

          const currentPath = wizardState.destDir
            ? wizardState.destDir.trim()
            : "";
          const expandedPath = expandPath(currentPath);

          if (expandedPath !== wizardState.destDir) {
            updateWizardState({ destDir: expandedPath });
          }

          const isValid = expandedPath ? fs.existsSync(expandedPath) : false;

          if (!expandedPath) {
            setError("WoW AddOns directory is required to continue.");
            setDirEditMode(true);
            return;
          }

          if (!isValid) {
            setPathValid(false);
            setError(
              "Invalid directory. Please provide a valid WoW AddOns path.",
            );
            setDirEditMode(true);
            return;
          }

          setPathValid(true);
          setError(null);
          goNext();
        }
        break;

      case 3: // Addons
        if (key.upArrow || input === "k") {
          flashKey("↑/↓");
          setAddonIndex(addonIndex === 0 ? 1 : 0);
        }
        if (key.downArrow || input === "j") {
          flashKey("↑/↓");
          setAddonIndex(addonIndex === 1 ? 0 : 1);
        }
        if (input === " ") {
          flashKey("space");
          if (addonIndex === 0) {
            updateWizardState({ installElvUI: !wizardState.installElvUI });
          } else {
            updateWizardState({ installTukui: !wizardState.installTukui });
          }
        }
        if (key.return) {
          flashKey("enter");
          goNext();
        }
        break;

      case 4: // Settings
        if (key.upArrow || input === "k") {
          flashKey("↑/↓");
          setSettingsIndex(Math.max(0, settingsIndex - 1));
        }
        if (key.downArrow || input === "j") {
          flashKey("↑/↓");
          setSettingsIndex(Math.min(4, settingsIndex + 1));
        }
        if (key.leftArrow || input === "h") {
          flashKey("←/→");
          handleSettingsAdjust(-1);
        }
        if (key.rightArrow || input === "l") {
          flashKey("←/→");
          handleSettingsAdjust(1);
        }
        if (input === " ") {
          flashKey("space");
          handleSettingsToggle();
        }
        if (key.return) {
          flashKey("enter");
          goNext();
        }
        break;

      case 5: // Review
        if (key.return) {
          flashKey("enter");
          handleComplete();
        }
        break;
    }
  });

  const handleSettingsAdjust = (delta: number) => {
    switch (settingsIndex) {
      case 0: // Max Concurrent
        updateWizardState({
          maxConcurrent: Math.max(
            1,
            Math.min(10, wizardState.maxConcurrent + delta),
          ),
        });
        break;
      case 1: // Check Interval
        {
          const intervals = [60, 120, 180, 300, 600, 900];
          const currentIdx = intervals.indexOf(wizardState.checkInterval);
          const newIdx = Math.max(
            0,
            Math.min(intervals.length - 1, currentIdx + delta),
          );
          updateWizardState({ checkInterval: intervals[newIdx] ?? 300 });
        }
        break;
      case 2: // Auto-check interval (if enabled)
        if (wizardState.autoCheckEnabled) {
          const intervals = [30, 60, 90, 120, 180, 240];
          const currentIdx = intervals.indexOf(wizardState.autoCheckInterval);
          const newIdx = Math.max(
            0,
            Math.min(intervals.length - 1, currentIdx + delta),
          );
          updateWizardState({ autoCheckInterval: intervals[newIdx] ?? 60 });
        }
        break;
      case 4: // Backup Retention
        updateWizardState({
          backupRetention: Math.max(
            1,
            Math.min(20, wizardState.backupRetention + delta),
          ),
        });
        break;
    }
  };

  const handleSettingsToggle = () => {
    switch (settingsIndex) {
      case 2: // Auto-check
        updateWizardState({ autoCheckEnabled: !wizardState.autoCheckEnabled });
        break;
      case 3: // Backup WTF
        updateWizardState({ backupWTF: !wizardState.backupWTF });
        break;
    }
  };

  const getControls = () => {
    const controls = [];
    if (step > 1) {
      controls.push({ key: "esc", label: "back" });
    } else {
      controls.push({ key: "esc", label: "exit" });
    }

    switch (step) {
      case 1:
        controls.push({ key: "←/→", label: "toggle" });
        break;
      case 2:
        controls.push({ key: "↑/↓", label: "switch mode" });
        break;
      case 3:
        controls.push({ key: "↑/↓", label: "nav" });
        controls.push({ key: "space", label: "toggle" });
        break;
      case 4:
        controls.push({ key: "↑/↓", label: "nav" });
        controls.push({ key: "←/→", label: "adjust" });
        controls.push({ key: "space", label: "toggle" });
        break;
    }

    controls.push({
      key: "enter",
      label: step === TOTAL_STEPS ? "confirm" : "next",
    });
    return controls;
  };

  return (
    <Box
      flexDirection="column"
      padding={1}
      borderStyle="round"
      borderColor={theme.wizardBorder}
    >
      <Box marginBottom={1} justifyContent="space-between">
        <Color styles={theme.brand}>
          <Text bold>🍋 LemonUp Setup</Text>
        </Color>
        <Color styles={theme.highlight}>
          <Text>
            Step {step} of {TOTAL_STEPS}
          </Text>
        </Color>
      </Box>

      <WizardProgress currentStep={step} theme={theme} />

      <Box flexDirection="column" minHeight={10} paddingX={1}>
        {step === 1 && <ThemeStep value={wizardState.theme} theme={theme} />}
        {step === 2 && (
          <DirectoryStep
            mode={wizardState.destDirMode}
            destDir={wizardState.destDir}
            onDirChange={(d) => updateWizardState({ destDir: d })}
            isEditing={dirEditMode}
            onEditToggle={setDirEditMode}
            pathValid={pathValid}
            theme={theme}
            isScanning={isScanning}
            scanError={scanError}
          />
        )}
        {step === 3 && (
          <AddonsStep
            installElvUI={wizardState.installElvUI}
            installTukui={wizardState.installTukui}
            selectedIndex={addonIndex}
            theme={theme}
          />
        )}
        {step === 4 && (
          <SettingsStep
            state={wizardState}
            selectedIndex={settingsIndex}
            theme={theme}
          />
        )}
        {step === 5 && <ReviewStep state={wizardState} theme={theme} />}
      </Box>

      {error && (
        <Box marginTop={1}>
          <Color styles={theme.error}>
            <Text>Error: {error}</Text>
          </Color>
        </Box>
      )}

      <ControlBar controls={getControls()} />
    </Box>
  );
};
