import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Box, Text, useApp, useInput } from "ink";
import Color from "ink-color-pipe";
import Spinner from "ink-spinner";
import {
  type ProgressContext,
  Step,
  type StepContext,
  Stepper,
} from "ink-stepper";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useRef, useState } from "react";
import type { ConfigManager } from "@/core/config";
import { logger } from "@/core/logger";
import {
  getDefaultWoWPath,
  quickCheckCommonPaths,
  searchForWoW,
} from "@/core/paths";
import {
  DEFAULT_EXPORT_PATH,
  type ExportedAddon,
  type ExportFile,
  parseImportFile,
} from "@/core/transfer";
import { ControlBar } from "./components/ControlBar";
import { useScanPathInput } from "./hooks/useScanPathInput";
import { useTheme } from "./hooks/useTheme";
import { useAppStore } from "./store/useAppStore";

interface FirstRunWizardProps {
  configManager: ConfigManager;
  onComplete: () => void;
}

interface WizardState {
  theme: "dark" | "light";
  destDir: string;
  installElvUI: boolean;
  installTukui: boolean;
  maxConcurrent: number;
  checkInterval: number; // seconds
  autoCheckEnabled: boolean;
  autoCheckInterval: number; // minutes
  backupWTF: boolean;
  backupRetention: number;
  importAddons: boolean; // Whether to import addons after wizard
}

const expandPath = (p: string): string => {
  if (p.startsWith("~/") || p === "~") {
    return path.join(os.homedir(), p.slice(1));
  }
  return p;
};

// Progress bar component adapted for ink-stepper's ProgressContext
const WizardProgress: React.FC<{
  progressContext: ProgressContext;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ progressContext, theme }) => {
  const { steps } = progressContext;
  const segmentWidth = 12;
  const halfSegment = Math.floor(segmentWidth / 2);

  // Build the progress bar with markers embedded
  const renderProgressBar = () => {
    const elements: React.ReactNode[] = [];

    for (let i = 0; i < steps.length; i++) {
      const step = steps[i]!;
      const isCompleted = step.completed;
      const isCurrent = step.current;
      const segmentColor =
        isCompleted || isCurrent
          ? theme.progressCompleted
          : theme.progressPending;

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

      // Add trailing half-segment (after marker), except for last step
      if (i < steps.length - 1) {
        const trailingColor = isCompleted
          ? theme.progressCompleted
          : theme.progressPending;
        elements.push(
          <Color key={`post-${i}`} styles={trailingColor}>
            <Text>{"━".repeat(segmentLen)}</Text>
          </Color>,
        );
      }
    }

    return elements;
  };

  return (
    <Box flexDirection="column" marginBottom={1} paddingLeft={1}>
      {/* Progress bar line with markers */}
      <Box>{renderProgressBar()}</Box>

      {/* Labels row - evenly spaced under markers */}
      <Box>
        {steps.map((step) => {
          const stepColor = step.current
            ? theme.progressCurrent
            : step.completed
              ? theme.progressCompleted
              : theme.progressPending;
          return (
            <Box
              key={step.name}
              width={segmentWidth + 1}
              justifyContent="center"
            >
              <Color styles={stepColor}>
                <Text bold={step.current}>{step.name}</Text>
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
  detectedPath: string;
  destDir: string;
  onDirChange: (d: string) => void;
  pathValid: boolean | null;
  theme: ReturnType<typeof useTheme>["theme"];
  isScanning: boolean;
  scanError: string | null;
  scanProgress: { dirs: number; path: string };
  scanPathInput: ReturnType<typeof useScanPathInput>;
  onStartScan: () => void;
  // Failure flow state
  scanSubstep: "choose" | "input";
  scanOptionIndex: number;
  // Success flow state
  successSubstep: "choose" | "input";
  successOptionIndex: number;
}> = ({
  detectedPath,
  destDir,
  onDirChange,
  pathValid,
  theme,
  isScanning,
  scanError,
  scanProgress,
  scanPathInput,
  onStartScan,
  scanSubstep,
  scanOptionIndex,
  successSubstep,
  successOptionIndex,
}) => {
  const isDetected = detectedPath !== "NOT_CONFIGURED";

  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Where is your WoW AddOns folder?</Text>
      </Color>

      <Box marginTop={1} flexDirection="column">
        {isDetected ? (
          /* ===== SUCCESS FLOW: Auto-detect succeeded ===== */
          <Box flexDirection="column">
            <Box>
              <Color styles={theme.success}>
                <Text>✓ Detected: </Text>
              </Color>
              <Color styles={theme.labelInactive}>
                <Text>{detectedPath}</Text>
              </Color>
            </Box>

            <Box flexDirection="column" marginTop={1}>
              <Box>
                <Color
                  styles={
                    successOptionIndex === 0 ? theme.selection : theme.muted
                  }
                >
                  <Text>{successOptionIndex === 0 ? "› " : "  "}</Text>
                </Color>
                <Color
                  styles={
                    successOptionIndex === 0
                      ? theme.highlight
                      : theme.labelInactive
                  }
                >
                  <Text bold={successOptionIndex === 0}>Use this path</Text>
                </Color>
              </Box>
              <Box>
                <Color
                  styles={
                    successOptionIndex === 1 ? theme.selection : theme.muted
                  }
                >
                  <Text>{successOptionIndex === 1 ? "› " : "  "}</Text>
                </Color>
                <Color
                  styles={
                    successOptionIndex === 1
                      ? theme.highlight
                      : theme.labelInactive
                  }
                >
                  <Text bold={successOptionIndex === 1}>
                    Enter different path
                  </Text>
                </Color>
              </Box>
            </Box>

            {/* Manual input when "Enter different path" selected */}
            {successSubstep === "input" && (
              <Box marginTop={1} marginLeft={2} flexDirection="column">
                <Box>
                  <Color styles={theme.brand}>
                    <Text bold>✎ </Text>
                  </Color>
                  <TextInput
                    value={destDir}
                    onChange={onDirChange}
                    placeholder="Enter path to WoW AddOns folder..."
                  />
                </Box>
                {destDir && pathValid !== null && (
                  <Box marginTop={1}>
                    <Color styles={pathValid ? theme.success : theme.error}>
                      <Text>
                        {pathValid ? "✓ Valid path" : "✗ Invalid path"}
                      </Text>
                    </Color>
                  </Box>
                )}
                <Box marginTop={1}>
                  <Color styles={theme.muted}>
                    <Text>(Press Enter to validate, Esc to go back)</Text>
                  </Color>
                </Box>
              </Box>
            )}
          </Box>
        ) : (
          /* ===== FAILURE FLOW: Auto-detect failed ===== */
          <Box flexDirection="column">
            <Color styles={theme.warning}>
              <Text>Could not auto-detect WoW path.</Text>
            </Color>

            <Box marginTop={1}>
              {isScanning ? (
                <Box flexDirection="column">
                  <Box>
                    <Color styles={theme.brand}>
                      {/* @ts-expect-error ink-spinner types are not fully compatible with React 19 */}
                      <Spinner type="dots" />
                    </Color>
                    <Text>
                      {" "}
                      Scanning... ({scanProgress.dirs} directories checked)
                    </Text>
                  </Box>
                  {scanProgress.path && (
                    <Box marginLeft={2}>
                      <Color styles={theme.muted}>
                        <Text>{scanProgress.path}</Text>
                      </Color>
                    </Box>
                  )}
                  <Box marginTop={1}>
                    <Color styles={theme.muted}>
                      <Text>(Press Esc to cancel)</Text>
                    </Color>
                  </Box>
                </Box>
              ) : (
                <Box flexDirection="column">
                  {/* Option Selection */}
                  <Box flexDirection="column">
                    <Box>
                      <Color
                        styles={
                          scanOptionIndex === 0 ? theme.selection : theme.muted
                        }
                      >
                        <Text>{scanOptionIndex === 0 ? "› " : "  "}</Text>
                      </Color>
                      <Color
                        styles={
                          scanOptionIndex === 0
                            ? theme.highlight
                            : theme.labelInactive
                        }
                      >
                        <Text bold={scanOptionIndex === 0}>Manual input</Text>
                      </Color>
                    </Box>
                    <Box>
                      <Color
                        styles={
                          scanOptionIndex === 1 ? theme.selection : theme.muted
                        }
                      >
                        <Text>{scanOptionIndex === 1 ? "› " : "  "}</Text>
                      </Color>
                      <Color
                        styles={
                          scanOptionIndex === 1
                            ? theme.highlight
                            : theme.labelInactive
                        }
                      >
                        <Text bold={scanOptionIndex === 1}>Deep Scan</Text>
                      </Color>
                    </Box>
                    {/* Help hint for Deep Scan */}
                    {scanOptionIndex === 1 && scanSubstep === "choose" && (
                      <Box marginLeft={4}>
                        <Color styles={theme.muted}>
                          <Text>Scan start path can be customized</Text>
                        </Color>
                      </Box>
                    )}
                  </Box>

                  {/* Manual input expanded */}
                  {scanSubstep === "input" && scanOptionIndex === 0 && (
                    <Box marginTop={1} marginLeft={2} flexDirection="column">
                      <Box>
                        <Color styles={theme.brand}>
                          <Text bold>✎ </Text>
                        </Color>
                        <TextInput
                          value={destDir}
                          onChange={onDirChange}
                          placeholder="Enter path to WoW AddOns folder..."
                        />
                      </Box>
                      {destDir && pathValid !== null && (
                        <Box marginTop={1}>
                          <Color
                            styles={pathValid ? theme.success : theme.error}
                          >
                            <Text>
                              {pathValid ? "✓ Valid path" : "✗ Invalid path"}
                            </Text>
                          </Color>
                        </Box>
                      )}
                      <Box marginTop={1}>
                        <Color styles={theme.muted}>
                          <Text>(Press Enter to validate, Esc to go back)</Text>
                        </Color>
                      </Box>
                    </Box>
                  )}

                  {/* Deep Scan expanded */}
                  {scanSubstep === "input" && scanOptionIndex === 1 && (
                    <Box flexDirection="column" marginLeft={2} marginTop={1}>
                      <Box>
                        <Color styles={theme.muted}>
                          <Text>Scan from: </Text>
                        </Color>
                        <TextInput
                          value={scanPathInput.inputValue}
                          onChange={scanPathInput.setInputValue}
                          onSubmit={onStartScan}
                          placeholder="Enter path..."
                        />
                      </Box>

                      {scanPathInput.suggestions.length > 0 && (
                        <Box flexDirection="column" marginTop={1}>
                          <Color styles={theme.muted}>
                            <Text>Suggestions (↑/↓ to select):</Text>
                          </Color>
                          {scanPathInput.suggestions.map((suggestion, idx) => (
                            <Box key={suggestion}>
                              <Color
                                styles={
                                  idx === scanPathInput.selectedIndex
                                    ? theme.selection
                                    : theme.muted
                                }
                              >
                                <Text>
                                  {idx === scanPathInput.selectedIndex
                                    ? "› "
                                    : "  "}
                                </Text>
                              </Color>
                              <Color
                                styles={
                                  idx === scanPathInput.selectedIndex
                                    ? theme.highlight
                                    : theme.labelInactive
                                }
                              >
                                <Text>{suggestion}</Text>
                              </Color>
                            </Box>
                          ))}
                        </Box>
                      )}

                      <Box marginTop={1}>
                        <Color styles={theme.muted}>
                          <Text>(Press Enter to scan, Esc to go back)</Text>
                        </Color>
                      </Box>
                    </Box>
                  )}

                  {scanError && (
                    <Box marginTop={1}>
                      <Color styles={theme.error}>
                        <Text>Scan failed: {scanError}</Text>
                      </Color>
                    </Box>
                  )}
                </Box>
              )}
            </Box>
          </Box>
        )}
      </Box>
    </Box>
  );
};

// Step 3: Import from Export
const ImportStep: React.FC<{
  exportData: ExportFile | null;
  exportFileExists: boolean;
  importSelected: boolean;
  optionIndex: number;
  refreshFailed: boolean;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({
  exportData,
  exportFileExists,
  importSelected,
  optionIndex,
  refreshFailed,
  theme,
}) => {
  if (!exportFileExists || !exportData) {
    return (
      <Box flexDirection="column" gap={1}>
        <Color styles={theme.heading}>
          <Text bold>Import Addons</Text>
        </Color>

        <Box marginTop={1} flexDirection="column">
          <Color styles={theme.warning}>
            <Text>No export file found at:</Text>
          </Color>
          <Box marginLeft={2}>
            <Color styles={theme.labelInactive}>
              <Text>~/lemonup-addons.json</Text>
            </Color>
          </Box>
        </Box>

        <Box marginTop={1}>
          <Color styles={theme.muted}>
            <Text>If you've just copied the file, press </Text>
          </Color>
          <Color styles={theme.highlight}>
            <Text bold>[r]</Text>
          </Color>
          <Color styles={theme.muted}>
            <Text> to refresh.</Text>
          </Color>
        </Box>

        {refreshFailed && (
          <Box marginTop={1}>
            <Color styles={theme.error}>
              <Text>
                ✗ File not found. Make sure it exists at the path above.
              </Text>
            </Color>
          </Box>
        )}

        <Box marginTop={1}>
          <Color styles={theme.muted}>
            <Text>
              You can also import addons later from Settings → Import.
            </Text>
          </Color>
        </Box>
      </Box>
    );
  }

  const reinstallableCount = exportData.addons.filter(
    (a) => a.reinstallable,
  ).length;
  const manualCount = exportData.addons.filter((a) => !a.reinstallable).length;

  return (
    <Box flexDirection="column" gap={1}>
      <Color styles={theme.heading}>
        <Text bold>Import Addons from Previous Installation?</Text>
      </Color>

      <Box marginTop={1} flexDirection="column">
        <Color styles={theme.success}>
          <Text>Found export file with {exportData.addons.length} addons:</Text>
        </Color>
        <Box marginLeft={2} flexDirection="column">
          <Color styles={theme.labelInactive}>
            <Text>{reinstallableCount} can be reinstalled</Text>
          </Color>
          {manualCount > 0 && (
            <Color styles={theme.muted}>
              <Text>{manualCount} manual (will be skipped)</Text>
            </Color>
          )}
        </Box>
      </Box>

      <Box marginTop={1} flexDirection="column">
        <Box>
          <Color styles={optionIndex === 0 ? theme.selection : theme.muted}>
            <Text>{optionIndex === 0 ? "› " : "  "}</Text>
          </Color>
          <Color
            styles={optionIndex === 0 ? theme.highlight : theme.labelInactive}
          >
            <Text bold={optionIndex === 0}>Yes, import these addons</Text>
          </Color>
          {importSelected && optionIndex === 0 && (
            <Color styles={theme.success}>
              <Text> ✓</Text>
            </Color>
          )}
        </Box>
        <Box>
          <Color styles={optionIndex === 1 ? theme.selection : theme.muted}>
            <Text>{optionIndex === 1 ? "› " : "  "}</Text>
          </Color>
          <Color
            styles={optionIndex === 1 ? theme.highlight : theme.labelInactive}
          >
            <Text bold={optionIndex === 1}>No, start fresh</Text>
          </Color>
        </Box>
      </Box>

      <Box marginTop={1}>
        <Color styles={theme.muted}>
          <Text>Use ↑/↓ to select, Enter to confirm.</Text>
        </Color>
      </Box>
    </Box>
  );
};

// Step 4: Addon Selection
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

// Step 6: Review
const ReviewStep: React.FC<{
  state: WizardState;
  exportData: ExportFile | null;
  theme: ReturnType<typeof useTheme>["theme"];
}> = ({ state, exportData, theme }) => {
  const formatAutoInterval = (mins: number) => {
    if (mins < 60) return `${mins}m`;
    const hrs = Math.floor(mins / 60);
    return `${hrs}hr`;
  };

  const addons = [];
  if (state.installElvUI) addons.push("ElvUI");
  if (state.installTukui) addons.push("Tukui");

  // Calculate import count
  const importCount =
    state.importAddons && exportData
      ? exportData.addons.filter((a) => a.reinstallable).length
      : 0;

  const items = [
    { label: "Theme", value: state.theme === "dark" ? "Dark" : "Light" },
    { label: "AddOns Directory", value: state.destDir || "(not set)" },
    {
      label: "Import",
      value: state.importAddons ? `Yes (${importCount} addons)` : "No",
    },
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
  const setImportQueue = useAppStore((state) => state.setImportQueue);

  // Ref to hold the current StepContext from ink-stepper
  const stepContextRef = useRef<StepContext | null>(null);

  // Local step state for keyboard handling (synced with stepper)
  const [step, setStep] = useState(1);
  const [error, setError] = useState<string | null>(null);

  // Deep Scan State
  const [isScanning, setIsScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [scanProgress, setScanProgress] = useState({ dirs: 0, path: "" });
  const scanAbortController = useRef<AbortController | null>(null);

  // Wizard state
  const [wizardState, setWizardState] = useState<WizardState>(() => ({
    theme: "dark",
    destDir: "", // Set when user confirms path
    installElvUI: false,
    installTukui: false,
    maxConcurrent: 3,
    checkInterval: 300, // 5 minutes in seconds
    autoCheckEnabled: true,
    autoCheckInterval: 60, // 1 hour in minutes
    backupWTF: true,
    backupRetention: 5,
    importAddons: false,
  }));

  // Import step state
  const [exportFileExists, setExportFileExists] = useState(false);
  const [exportData, setExportData] = useState<ExportFile | null>(null);
  const [importOptionIndex, setImportOptionIndex] = useState(0); // 0 = Import, 1 = Skip
  const [importRefreshFailed, setImportRefreshFailed] = useState(false);

  // Step-specific state
  const [dirEditMode, setDirEditMode] = useState(false);
  const [pathValid, setPathValid] = useState<boolean | null>(null);
  const [addonIndex, setAddonIndex] = useState(0);
  const [settingsIndex, setSettingsIndex] = useState(0);

  // Scan options substep: "choose" = selecting Manual/DeepScan, "input" = entering path/scan
  const [scanSubstep, setScanSubstep] = useState<"choose" | "input">("choose");
  const [scanOptionIndex, setScanOptionIndex] = useState(0); // 0 = Manual, 1 = Deep Scan

  // Success flow state (auto-detect succeeded)
  const [successOptionIndex, setSuccessOptionIndex] = useState(0); // 0 = Use this, 1 = Different
  const [successSubstep, setSuccessSubstep] = useState<"choose" | "input">(
    "choose",
  );
  const [detectedPath, setDetectedPath] = useState<string>("NOT_CONFIGURED");

  const updateWizardState = (updates: Partial<WizardState>) => {
    setWizardState((prev) => ({ ...prev, ...updates }));
  };

  // Scan path input hook
  const scanPathInput = useScanPathInput();

  // Auto-detect WoW path on mount
  useEffect(() => {
    getDefaultWoWPath().then((detected) => {
      setDetectedPath(detected);
      if (detected !== "NOT_CONFIGURED") {
        setPathValid(true);
      }
    });
  }, []);

  // Check for existing export file on mount
  useEffect(() => {
    const checkExportFile = async () => {
      if (fs.existsSync(DEFAULT_EXPORT_PATH)) {
        const result = await parseImportFile(DEFAULT_EXPORT_PATH);
        if (result.success && result.data) {
          setExportFileExists(true);
          setExportData(result.data);
        }
      }
    };
    checkExportFile();
  }, []);

  const handleDeepScan = async () => {
    setIsScanning(true);
    setScanError(null);
    setScanProgress({ dirs: 0, path: "" });
    scanAbortController.current = new AbortController();

    try {
      const scanRoot = scanPathInput.getSelectedPath();

      // Phase 1: Quick check common paths (no traversal)
      const quickResult = await quickCheckCommonPaths(scanRoot);
      if (quickResult) {
        updateWizardState({ destDir: quickResult });
        setPathValid(true);
        setIsScanning(false);
        return;
      }

      // Phase 2: Deep scan if quick check fails
      const result = await searchForWoW(
        scanRoot,
        scanAbortController.current.signal,
        (dirsScanned, currentPath) => {
          setScanProgress({ dirs: dirsScanned, path: currentPath });
        },
      );

      if (result && result !== "NOT_CONFIGURED") {
        updateWizardState({ destDir: result });
        setPathValid(true);
        setDetectedPath(result); // Switch UI to success flow
      } else {
        setScanError("No WoW installation found in deep scan.");
      }
    } catch (e: unknown) {
      if (e instanceof Error && e.name === "AbortError") {
        // User cancelled - ignore
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
      // Preserve debug logging state (may have been enabled via existing config)
      const wasDebugEnabled = logger.isEnabled();

      // Create config with wizard values
      configManager.createDefaultConfig();

      // Restore debug logging if it was previously enabled
      if (wasDebugEnabled) {
        configManager.set("debug", true);
      }

      // Apply wizard settings
      configManager.set("theme", wizardState.theme);
      configManager.set("destDir", wizardState.destDir);
      configManager.set("maxConcurrent", wizardState.maxConcurrent);
      configManager.set("checkInterval", wizardState.checkInterval * 1000);
      configManager.set("autoCheckEnabled", wizardState.autoCheckEnabled);
      configManager.set(
        "autoCheckInterval",
        wizardState.autoCheckInterval * 60 * 1000,
      );
      configManager.set("backupWTF", wizardState.backupWTF);
      configManager.set("backupRetention", wizardState.backupRetention);

      // Queue TukUI addons if selected
      const tukuiAddons: ExportedAddon[] = [];
      if (wizardState.installElvUI) {
        tukuiAddons.push({
          name: "ElvUI",
          folder: "ElvUI",
          type: "tukui",
          url: "latest",
          ownedFolders: ["ElvUI_Options", "ElvUI_Libraries"],
          reinstallable: true,
        });
      }
      if (wizardState.installTukui) {
        tukuiAddons.push({
          name: "Tukui",
          folder: "Tukui",
          type: "tukui",
          url: "latest",
          ownedFolders: [],
          reinstallable: true,
        });
      }

      // Queue imports if selected (combine with TukUI addons)
      const importAddons =
        wizardState.importAddons && exportData
          ? exportData.addons.filter((a) => a.reinstallable)
          : [];

      const allAddons = [...tukuiAddons, ...importAddons];
      if (allAddons.length > 0) {
        setImportQueue(allAddons);
      }

      onComplete();
    } catch (e) {
      setError(String(e));
    }
  };

  const TOTAL_STEPS = 6;

  // Navigation helpers that use ink-stepper's StepContext
  // Note: Local step state is synced via captureContext on re-render
  const goNext = () => {
    setError(null);
    stepContextRef.current?.goNext();
  };

  const goBack = () => {
    setError(null);
    if (stepContextRef.current?.isFirst) {
      exit();
    } else {
      stepContextRef.current?.goBack();
    }
  };

  useInput((input, key) => {
    // Handle escape/backspace for back navigation
    if (key.escape || key.backspace) {
      if (isScanning && scanAbortController.current) {
        scanAbortController.current.abort();
        setIsScanning(false);
        flashKey("esc");
        return;
      }

      // Step 2: Handle Esc for substep navigation
      if (step === 2) {
        // If editing, exit edit mode and reset substep
        if (dirEditMode) {
          flashKey("esc");
          setError(null);
          setDirEditMode(false);
          // Reset substep for both flows
          if (successSubstep === "input") {
            setSuccessSubstep("choose");
          }
          if (scanSubstep === "input") {
            setScanSubstep("choose");
          }
          return;
        }

        // If in scan input substep (failure flow), go back to choose
        if (scanSubstep === "input") {
          flashKey("esc");
          setScanSubstep("choose");
          return;
        }

        // If in success input substep, go back to choose
        if (successSubstep === "input") {
          flashKey("esc");
          setSuccessSubstep("choose");
          return;
        }
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

      case 2: {
        // Directory
        const isDetected = detectedPath !== "NOT_CONFIGURED";

        // ===== SUCCESS FLOW: Auto-detect succeeded =====
        if (isDetected) {
          // In text input mode
          if (dirEditMode) {
            if (key.return) {
              flashKey("enter");
              const currentPath = wizardState.destDir?.trim() || "";
              const expandedPath = expandPath(currentPath);

              if (expandedPath !== wizardState.destDir) {
                updateWizardState({ destDir: expandedPath });
              }

              const isValid = expandedPath
                ? fs.existsSync(expandedPath)
                : false;

              if (!expandedPath) {
                setError("WoW AddOns directory is required to continue.");
                return;
              }

              if (!isValid) {
                setPathValid(false);
                setError(
                  "Invalid directory. Please provide a valid WoW AddOns path.",
                );
                return;
              }

              setPathValid(true);
              setError(null);
              setDirEditMode(false);
              goNext();
            }
            return; // TextInput handles other keys
          }

          // Arrow navigation between options
          if (key.upArrow || key.downArrow || input === "j" || input === "k") {
            if (successSubstep === "choose") {
              flashKey("↑/↓");
              setSuccessOptionIndex((prev) => (prev === 0 ? 1 : 0));
              return;
            }
          }

          // Enter to select option
          if (key.return) {
            flashKey("enter");
            if (successSubstep === "choose") {
              if (successOptionIndex === 0) {
                // Use detected path - set destDir and proceed
                updateWizardState({ destDir: detectedPath });
                goNext();
              } else {
                // Enter different path - show input
                setSuccessSubstep("input");
                setDirEditMode(true);
              }
              return;
            }
          }
          break;
        }

        // ===== FAILURE FLOW: Auto-detect failed =====
        if (!isScanning) {
          // In text input mode (Manual input selected)
          if (dirEditMode) {
            if (key.return) {
              flashKey("enter");
              const currentPath = wizardState.destDir?.trim() || "";
              const expandedPath = expandPath(currentPath);

              if (expandedPath !== wizardState.destDir) {
                updateWizardState({ destDir: expandedPath });
              }

              const isValid = expandedPath
                ? fs.existsSync(expandedPath)
                : false;

              if (!expandedPath) {
                setError("WoW AddOns directory is required to continue.");
                return;
              }

              if (!isValid) {
                setPathValid(false);
                setError(
                  "Invalid directory. Please provide a valid WoW AddOns path.",
                );
                return;
              }

              setPathValid(true);
              setError(null);
              setDirEditMode(false);
              goNext();
            }
            return; // TextInput handles other keys
          }

          // Arrow keys
          if (key.upArrow || key.downArrow || input === "j" || input === "k") {
            flashKey("↑/↓");
            setError(null);

            if (scanSubstep === "choose") {
              // Navigate between Manual/DeepScan options
              setScanOptionIndex((prev) => (prev === 0 ? 1 : 0));
              return;
            }

            if (scanSubstep === "input" && scanOptionIndex === 1) {
              // Navigate suggestions in Deep Scan mode
              if (key.downArrow || input === "j") {
                const maxIndex = scanPathInput.suggestions.length - 1;
                scanPathInput.selectIndex(
                  Math.min(scanPathInput.selectedIndex + 1, maxIndex),
                );
              } else {
                scanPathInput.selectIndex(
                  Math.max(scanPathInput.selectedIndex - 1, -1),
                );
              }
              return;
            }
          }

          // Enter key
          if (key.return) {
            flashKey("enter");

            if (scanSubstep === "choose") {
              // Enter the selected option
              setScanSubstep("input");
              if (scanOptionIndex === 0) {
                // Manual input - show text input
                setDirEditMode(true);
              }
              return;
            }

            // Deep Scan Enter is handled by TextInput's onSubmit
          }
        }
        break;
      }

      case 3: // Import
        // Refresh check with 'r' key
        if (input === "r") {
          flashKey("r");
          // Re-check for export file
          if (fs.existsSync(DEFAULT_EXPORT_PATH)) {
            parseImportFile(DEFAULT_EXPORT_PATH).then((result) => {
              if (result.success && result.data) {
                setExportFileExists(true);
                setExportData(result.data);
                setImportRefreshFailed(false);
              } else {
                setImportRefreshFailed(true);
              }
            });
          } else {
            setImportRefreshFailed(true);
          }
          break;
        }

        // If no export file, just proceed on Enter
        if (!exportFileExists) {
          if (key.return) {
            flashKey("enter");
            goNext();
          }
          break;
        }

        // Arrow navigation
        if (key.upArrow || key.downArrow || input === "j" || input === "k") {
          flashKey("↑/↓");
          setImportOptionIndex((prev) => (prev === 0 ? 1 : 0));
        }

        // Enter to confirm selection
        if (key.return) {
          flashKey("enter");
          if (importOptionIndex === 0) {
            // Import selected
            updateWizardState({ importAddons: true });
          } else {
            // Skip import
            updateWizardState({ importAddons: false });
          }
          goNext();
        }
        break;

      case 4: // Addons
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

      case 5: // Settings
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

      case 6: // Review
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
      case 3: // Import
        if (exportFileExists) {
          controls.push({ key: "↑/↓", label: "select" });
        } else {
          controls.push({ key: "r", label: "refresh" });
        }
        break;
      case 4: // Addons
        controls.push({ key: "↑/↓", label: "nav" });
        controls.push({ key: "space", label: "toggle" });
        break;
      case 5: // Settings
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

  // Helper to capture stepContext for navigation calls
  const captureContext = (ctx: StepContext) => {
    stepContextRef.current = ctx;
  };

  // Handle step changes from ink-stepper (step is 0-indexed, we use 1-indexed)
  const handleStepChange = (newStep: number) => {
    setStep(newStep + 1);
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

      <Stepper
        onComplete={handleComplete}
        onCancel={exit}
        onStepChange={handleStepChange}
        keyboardNav={false}
        renderProgress={(progressContext) => (
          <WizardProgress progressContext={progressContext} theme={theme} />
        )}
      >
        <Step name="Theme">
          {(ctx) => {
            captureContext(ctx);
            return <ThemeStep value={wizardState.theme} theme={theme} />;
          }}
        </Step>
        <Step name="Directory">
          {(ctx) => {
            captureContext(ctx);
            return (
              <DirectoryStep
                detectedPath={detectedPath}
                destDir={wizardState.destDir}
                onDirChange={(d) => updateWizardState({ destDir: d })}
                pathValid={pathValid}
                theme={theme}
                isScanning={isScanning}
                scanError={scanError}
                scanProgress={scanProgress}
                scanPathInput={scanPathInput}
                onStartScan={handleDeepScan}
                scanSubstep={scanSubstep}
                scanOptionIndex={scanOptionIndex}
                successSubstep={successSubstep}
                successOptionIndex={successOptionIndex}
              />
            );
          }}
        </Step>
        <Step name="Import">
          {(ctx) => {
            captureContext(ctx);
            return (
              <ImportStep
                exportData={exportData}
                exportFileExists={exportFileExists}
                importSelected={wizardState.importAddons}
                optionIndex={importOptionIndex}
                refreshFailed={importRefreshFailed}
                theme={theme}
              />
            );
          }}
        </Step>
        <Step name="TukUI">
          {(ctx) => {
            captureContext(ctx);
            return (
              <AddonsStep
                installElvUI={wizardState.installElvUI}
                installTukui={wizardState.installTukui}
                selectedIndex={addonIndex}
                theme={theme}
              />
            );
          }}
        </Step>
        <Step name="Settings">
          {(ctx) => {
            captureContext(ctx);
            return (
              <SettingsStep
                state={wizardState}
                selectedIndex={settingsIndex}
                theme={theme}
              />
            );
          }}
        </Step>
        <Step name="Review">
          {(ctx) => {
            captureContext(ctx);
            return (
              <ReviewStep
                state={wizardState}
                exportData={exportData}
                theme={theme}
              />
            );
          }}
        </Step>
      </Stepper>

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
