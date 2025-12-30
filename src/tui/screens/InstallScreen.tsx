import { Box, Text, useInput } from "ink";
import Color from "ink-color-pipe";
import Spinner from "ink-spinner";
import TextInput from "ink-text-input";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import type { Config } from "@/core/config";
import type { AddonManager } from "@/core/manager";
import { getDefaultWoWPath, isPathConfigured, pathExists } from "@/core/paths";
import type { ExportedAddon } from "@/core/transfer";
import { ControlBar } from "@/tui/components/ControlBar";
import { ScreenTitle } from "@/tui/components/ScreenTitle";
import { useTheme } from "@/tui/hooks/useTheme";
import { useAppStore } from "@/tui/store/useAppStore";

interface InstallScreenProps {
  config: Config;
  addonManager: AddonManager;
  onBack: () => void;
}

type Mode =
  | "select"
  | "url-input"
  | "installing"
  | "result"
  | "config-auto-confirm"
  | "config-manual-input"
  | "confirm-reinstall"
  | "batch-installing"
  | "batch-result";

export const InstallScreen: React.FC<InstallScreenProps> = ({
  config: initialConfig,
  addonManager,
  onBack,
}) => {
  const { theme } = useTheme();
  const flashKey = useAppStore((state) => state.flashKey);
  const importQueue = useAppStore((state) => state.importQueue);
  const clearImportQueue = useAppStore((state) => state.clearImportQueue);
  const [config, setConfig] = useState(initialConfig);
  const [mode, setMode] = useState<Mode>("select");
  const [selection, setSelection] = useState(0);
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState("");
  const [resultMessage, setResultMessage] = useState("");
  const [manualPath, setManualPath] = useState("");
  const [detectedPath, setDetectedPath] = useState("");

  const [pendingInstall, setPendingInstall] = useState<{
    type: "url" | "elvui" | "tukui";
    url?: string;
  } | null>(null);

  const [resultStatus, setResultStatus] = useState<"success" | "error">(
    "success",
  );

  // Batch import state
  type AddonStatus = "pending" | "installing" | "success" | "failed";
  type BatchAddon = {
    addon: ExportedAddon;
    status: AddonStatus;
    error?: string;
  };
  const [batchAddons, setBatchAddons] = useState<BatchAddon[]>([]);

  const OPTIONS = [
    { label: "Install from URL", action: "url", section: "General" },
    { label: "Install ElvUI", action: "elvui", section: "TukUI" },
    { label: "Install TukUI", action: "tukui", section: "TukUI" },
  ];

  useEffect(() => {
    setConfig(addonManager.getConfig());
  }, [addonManager]);

  // Process batch install from import queue with concurrency
  const processBatchInstall = useCallback(
    async (queue: ExportedAddon[]) => {
      clearImportQueue();

      // Initialize all addons as pending
      const initialBatch: BatchAddon[] = queue.map((addon) => ({
        addon,
        status: "pending" as AddonStatus,
      }));
      setBatchAddons(initialBatch);
      setMode("batch-installing");

      const maxConcurrent = config.maxConcurrent ?? 3;
      let currentIndex = 0;
      const results = [...initialBatch];

      const updateAddonStatus = (
        index: number,
        status: AddonStatus,
        error?: string,
      ) => {
        results[index] = { ...results[index], status, error } as BatchAddon;
        setBatchAddons([...results]);
      };

      const installAddon = async (index: number): Promise<void> => {
        const item = results[index];
        if (!item) return;

        const addon = item.addon;
        updateAddonStatus(index, "installing");

        try {
          if (addon.type === "github" || addon.type === "wowinterface") {
            if (!addon.url) throw new Error("No URL available");
            await addonManager.installFromUrl(addon.url);
          } else if (addon.type === "tukui") {
            const subFolders = addon.ownedFolders ?? [];
            await addonManager.installTukUI("latest", addon.folder, subFolders);
          }
          updateAddonStatus(index, "success");
        } catch (e) {
          updateAddonStatus(
            index,
            "failed",
            e instanceof Error ? e.message : String(e),
          );
        }
      };

      // Process with concurrency limit
      const workers: Promise<void>[] = [];

      const startNext = async (): Promise<void> => {
        while (currentIndex < queue.length) {
          const index = currentIndex++;
          await installAddon(index);
        }
      };

      // Start up to maxConcurrent workers
      for (let i = 0; i < Math.min(maxConcurrent, queue.length); i++) {
        workers.push(startNext());
      }

      await Promise.all(workers);
      setMode("batch-result");
    },
    [addonManager, clearImportQueue, config.maxConcurrent],
  );

  // Detect import queue and start batch install
  useEffect(() => {
    if (importQueue.length > 0 && mode === "select") {
      processBatchInstall([...importQueue]);
    }
  }, [importQueue, mode, processBatchInstall]);

  const checkConfigAndInstall = async (
    type: "url" | "elvui" | "tukui",
    installUrl?: string,
  ) => {
    let exists = false;
    if (type === "elvui") {
      exists = addonManager.isAlreadyInstalled("ElvUI");
    } else if (type === "tukui") {
      exists = addonManager.isAlreadyInstalled("Tukui");
    } else if (type === "url" && installUrl) {
      exists = addonManager.isAlreadyInstalled(installUrl);
    }

    if (exists) {
      setPendingInstall({ type, url: installUrl });
      setMode("confirm-reinstall");
      return;
    }

    if (!isPathConfigured(config.destDir)) {
      setPendingInstall({ type, url: installUrl });
      const def = await getDefaultWoWPath();
      setDetectedPath(def);
      setMode("config-auto-confirm");
    } else {
      await handleInstall(type, installUrl);
    }
  };

  const handleInstall = async (
    type: "url" | "elvui" | "tukui",
    installUrl?: string,
  ) => {
    setMode("installing");
    setStatus("Installing...");
    setResultStatus("success");

    try {
      if (type === "url") {
        if (!installUrl) throw new Error("No URL provided");
        setStatus(`Installing from ${installUrl}...`);
        const res = await addonManager.installFromUrl(installUrl);
        if (res.success) {
          setResultMessage(
            `Successfully installed: ${res.installedAddons.join(", ")}`,
          );
          setResultStatus("success");
        } else {
          setResultMessage(res.error || "Unknown Error");
          setResultStatus("error");
        }
      } else if (type === "elvui") {
        setStatus("Downloading ElvUI from TukUI...");
        await addonManager.installTukUI("latest", "ElvUI", [
          "ElvUI_Options",
          "ElvUI_Libraries",
        ]);

        setResultMessage("ElvUI Installed Successfully");
        setResultStatus("success");
      } else if (type === "tukui") {
        setStatus("Downloading TukUI...");
        await addonManager.installTukUI("latest", "Tukui", []);

        setResultMessage("TukUI Installed Successfully");
        setResultStatus("success");
      }
    } catch (e) {
      setResultMessage(e instanceof Error ? e.message : String(e));
      setResultStatus("error");
    }
    setMode("result");
  };

  const savePathAndRetry = async (pathStr: string) => {
    try {
      addonManager.setConfigValue("destDir", pathStr);
      // Update local config
      setConfig({ ...config, destDir: pathStr });

      // Retry pending install
      if (pendingInstall) {
        await handleInstall(pendingInstall.type, pendingInstall.url);
      } else {
        setMode("select");
      }
    } catch (e) {
      setResultMessage(`Failed to save config: ${String(e)}`);
      setResultStatus("error");
      setMode("result");
    }
  };

  useInput(async (input, key) => {
    if (mode === "installing" || mode === "batch-installing") return;

    if (mode === "batch-result") {
      if (key.return || key.escape || input === "q") {
        flashKey("enter");
        setMode("select");
        setBatchAddons([]);
      }
      return;
    }

    if (mode === "result") {
      if (key.return || key.escape || input === "q") {
        flashKey("enter");
        setUrl("");
        setMode("select");
      }
      return;
    }

    if (mode === "config-auto-confirm") {
      if (input.toLowerCase() === "y" || key.return) {
        flashKey("enter");
        await savePathAndRetry(detectedPath);
      } else if (input.toLowerCase() === "n" || key.escape) {
        flashKey("esc");
        setMode("config-manual-input");
      }
      return;
    }

    if (mode === "confirm-reinstall") {
      if (input.toLowerCase() === "y") {
        flashKey("enter");
        if (pendingInstall) {
          await handleInstall(pendingInstall.type, pendingInstall.url);
        }
      } else if (input.toLowerCase() === "n" || key.escape || key.return) {
        flashKey("esc");
        if (pendingInstall && pendingInstall.type === "url") {
          setMode("url-input");
          if (pendingInstall.url) setUrl(pendingInstall.url);
        } else {
          setMode("select");
        }
        setPendingInstall(null);
      }
      return;
    }

    if (mode === "config-manual-input") {
      if (key.return) {
        flashKey("enter");
        if (manualPath.trim()) {
          if (pathExists(manualPath.trim())) {
            await savePathAndRetry(manualPath.trim());
          } else {
            await savePathAndRetry(manualPath.trim());
          }
        }
      } else if (key.escape) {
        flashKey("esc");
        setMode("select");
        setPendingInstall(null);
      }
      return;
    }

    if (mode === "select") {
      if (key.upArrow) {
        setSelection((prev) => (prev > 0 ? prev - 1 : prev));
      } else if (key.downArrow) {
        setSelection((prev) => (prev < OPTIONS.length - 1 ? prev + 1 : prev));
      } else if (key.return) {
        flashKey("enter");
        const action = OPTIONS[selection]?.action;
        if (action === "url") {
          setMode("url-input");
        } else if (action === "elvui") {
          await checkConfigAndInstall("elvui");
        } else if (action === "tukui") {
          await checkConfigAndInstall("tukui");
        }
      } else if (key.escape || input === "q") {
        flashKey("esc");
        onBack();
      }
    } else if (mode === "url-input") {
      if (key.return) {
        flashKey("enter");
        if (url.trim()) {
          await checkConfigAndInstall("url", url);
        }
      } else if (key.escape) {
        flashKey("esc");
        setMode("select");
      }
    }
  });

  return (
    <Box flexDirection="column" gap={1}>
      <ScreenTitle title="Install Addon" />

      {mode === "config-auto-confirm" && (
        <Box
          flexDirection="column"
          borderColor="yellow"
          borderStyle="round"
          padding={1}
        >
          <Text color="yellow" bold>
            WoW Addon Directory Not Configured
          </Text>
          <Text>I detected a default location:</Text>
          <Text color="blue">{detectedPath}</Text>
          <Box marginTop={1}>
            <Text>Do you want to use this path? (Y/n)</Text>
          </Box>
        </Box>
      )}

      {mode === "confirm-reinstall" && (
        <Box
          flexDirection="column"
          borderColor="red"
          borderStyle="round"
          padding={1}
        >
          <Text color="red" bold>
            Warning: Addon Already Installed
          </Text>
          <Text>It looks like this addon is already installed.</Text>
          <Box marginTop={1}>
            <Text>Do you want to reinstall and overwrite it? (y/N)</Text>
          </Box>
        </Box>
      )}

      {mode === "config-manual-input" && (
        <Box
          flexDirection="column"
          borderColor="cyan"
          borderStyle="round"
          padding={1}
        >
          <Text bold>Enter WoW Addon Directory Path:</Text>
          <TextInput value={manualPath} onChange={setManualPath} />
          <Box marginTop={1}>
            <Text color="gray">Press Enter to save, Esc to cancel</Text>
          </Box>
        </Box>
      )}

      {mode === "select" && (
        <Box flexDirection="column">
          {(() => {
            let lastSection = "";
            return OPTIONS.map((opt, i) => {
              const showHeader = opt.section !== lastSection;
              lastSection = opt.section;
              const isSelected = i === selection;
              return (
                <Box flexDirection="column" key={opt.action}>
                  {showHeader && (
                    <Box marginTop={i > 0 ? 1 : 0} marginBottom={0}>
                      <Color styles={theme.statusWarning}>
                        <Text bold underline>
                          {opt.section}
                        </Text>
                      </Color>
                    </Box>
                  )}
                  <Color
                    styles={isSelected ? theme.highlight : theme.labelInactive}
                  >
                    <Text>
                      {isSelected ? "> " : "  "} {opt.label}
                    </Text>
                  </Color>
                </Box>
              );
            });
          })()}
        </Box>
      )}

      {mode === "url-input" && (
        <Box>
          <Text>URL: </Text>
          <TextInput value={url} onChange={setUrl} onSubmit={() => {}} />
        </Box>
      )}

      {mode === "installing" && (
        <Color styles={theme.busy}>
          <Text>
            {/* @ts-expect-error: Spinner types mismatch */}
            <Spinner type="dots" /> {status}
          </Text>
        </Color>
      )}

      {mode === "result" && (
        <Box flexDirection="column">
          <Color
            styles={resultStatus === "success" ? theme.success : theme.error}
          >
            <Text>
              {resultStatus === "success" ? "✔ " : "✘ "}
              {resultMessage}
            </Text>
          </Color>
          <Color styles={theme.muted}>
            <Text>Press Enter to continue</Text>
          </Color>
        </Box>
      )}

      {(mode === "batch-installing" || mode === "batch-result") && (
        <Box flexDirection="column">
          {mode === "batch-installing" && (
            <Color styles={theme.busy}>
              <Text bold>Installing addons from import...</Text>
            </Color>
          )}
          {mode === "batch-result" && (
            <Color styles={theme.statusSuccess}>
              <Text bold>
                Import Complete - Installed:{" "}
                {batchAddons.filter((a) => a.status === "success").length}/
                {batchAddons.length}
              </Text>
            </Color>
          )}
          <Box marginTop={1} flexDirection="column">
            {batchAddons.map((item) => (
              <Box key={item.addon.folder} gap={1}>
                {item.status === "pending" && (
                  <Color styles={theme.muted}>
                    <Text>○</Text>
                  </Color>
                )}
                {item.status === "installing" && (
                  <Color styles={theme.statusChecking}>
                    {/* @ts-expect-error: Spinner types mismatch */}
                    <Spinner type="dots" />
                  </Color>
                )}
                {item.status === "success" && (
                  <Color styles={theme.statusSuccess}>
                    <Text>✔</Text>
                  </Color>
                )}
                {item.status === "failed" && (
                  <Color styles={theme.statusError}>
                    <Text>✘</Text>
                  </Color>
                )}
                <Color
                  styles={
                    item.status === "installing"
                      ? theme.highlight
                      : item.status === "success"
                        ? theme.statusSuccess
                        : item.status === "failed"
                          ? theme.statusError
                          : theme.muted
                  }
                >
                  <Text>
                    {item.addon.name}
                    {item.error && ` - ${item.error}`}
                  </Text>
                </Color>
              </Box>
            ))}
          </Box>
          {mode === "batch-result" && (
            <Box marginTop={1}>
              <Color styles={theme.muted}>
                <Text>Press Enter to continue</Text>
              </Color>
            </Box>
          )}
        </Box>
      )}

      <ControlBar
        message={
          mode === "url-input" ? (
            <Text>Enter URL (GitHub/WoWInterface)</Text>
          ) : undefined
        }
        controls={[
          { key: "esc", label: "back/cancel" },
          ...(mode === "select" ? [{ key: "enter", label: "select" }] : []),
          ...(mode === "url-input" ? [{ key: "enter", label: "install" }] : []),
        ]}
      />
    </Box>
  );
};
