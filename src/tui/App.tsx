import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Box, Text, useStdout } from "ink";
import type React from "react";
import { useEffect, useState } from "react";
import { type Config, ConfigManager } from "@/core/config";
import { AddonManager } from "@/core/manager";
import { Header } from "./components/Header";
import { FirstRunWizard } from "./FirstRunWizard";
import { useProgressBar } from "./hooks/useProgressBar";
import { ConfigScreen } from "./screens/ConfigScreen";
import { InstallScreen } from "./screens/InstallScreen";
import { MainMenu } from "./screens/MainMenu";
import { ManageScreen } from "./screens/ManageScreen";
import { WagoSearchScreen } from "./screens/WagoSearchScreen";
import { useAppStore } from "./store/useAppStore";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 60 * 1000,
      gcTime: 5 * 60 * 1000,
    },
  },
});

interface AppProps {
  force?: boolean;
  dryRun?: boolean;
  testMode?: boolean;
}

export const App: React.FC<AppProps> = ({
  force = false,
  dryRun = false,
  testMode = false,
}) => {
  return (
    <QueryClientProvider client={queryClient}>
      <TerminalSizeGuard>
        <AppContent force={force} dryRun={dryRun} testMode={testMode} />
      </TerminalSizeGuard>
    </QueryClientProvider>
  );
};

const MIN_TERMINAL_WIDTH = 80;
const MIN_TERMINAL_HEIGHT = 20;

const TerminalSizeGuard: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const { stdout } = useStdout();
  const [dims, setDims] = useState({
    cols: stdout.columns ?? 80,
    rows: stdout.rows ?? 24,
  });

  useEffect(() => {
    const handler = () => {
      setDims({
        cols: stdout.columns ?? 80,
        rows: stdout.rows ?? 24,
      });
    };
    stdout.on("resize", handler);
    return () => {
      stdout.off("resize", handler);
    };
  }, [stdout]);

  if (dims.cols < MIN_TERMINAL_WIDTH || dims.rows < MIN_TERMINAL_HEIGHT) {
    return (
      <Box
        justifyContent="center"
        alignItems="center"
        height={dims.rows}
        width={dims.cols}
      >
        <Text bold color="#0077aa">
          Terminal too small ({dims.cols}x{dims.rows}). Minimum:{" "}
          {MIN_TERMINAL_WIDTH}x{MIN_TERMINAL_HEIGHT}
        </Text>
      </Box>
    );
  }

  return <>{children}</>;
};

const AppContent: React.FC<AppProps> = ({
  force = false,
  dryRun = false,
  testMode = false,
}) => {
  const activeScreen = useAppStore((state) => state.activeScreen);
  const isBusy = useAppStore((state) => state.isBusy);
  const lastMenuSelection = useAppStore((state) => state.lastMenuSelection);
  const navigate = useAppStore((state) => state.navigate);
  const setLastMenuSelection = useAppStore(
    (state) => state.setLastMenuSelection,
  );
  const setTheme = useAppStore((state) => state.setTheme);
  const theme = useAppStore((state) => state.theme);
  const setPendingUpdates = useAppStore((state) => state.setPendingUpdates);
  const setBackgroundChecking = useAppStore(
    (state) => state.setBackgroundChecking,
  );
  const setDevMode = useAppStore((state) => state.setDevMode);
  const setNextCheckTime = useAppStore((state) => state.setNextCheckTime);
  const showToast = useAppStore((state) => state.showToast);
  const showOnboarding = useAppStore((state) => state.showOnboarding);
  const clearOnboarding = useAppStore((state) => state.clearOnboarding);

  const [initialLoad, setInitialLoad] = useState(true);
  const [showWizard, setShowWizard] = useState(false);
  const [configManager, setConfigManager] = useState<ConfigManager | null>(
    null,
  );
  const [addonManager, setAddonManager] = useState<AddonManager | null>(null);
  const [config, setConfig] = useState<Config | null>(null);

  const progressBar = useProgressBar({
    enabled: config?.terminalProgress ?? true,
  });

  // Set dev mode in store when testMode prop is true
  useEffect(() => {
    if (testMode) {
      setDevMode(true);
    }
  }, [testMode, setDevMode]);

  useEffect(() => {
    if (configManager) return;

    let manager: ConfigManager;
    if (testMode) {
      const fs = require("node:fs");
      const path = require("node:path");
      const os = require("node:os");

      const tempConfigDir = path.join(os.tmpdir(), "lemonup-test-config");
      fs.mkdirSync(tempConfigDir, { recursive: true });

      const configFile = path.join(tempConfigDir, "config.json");

      if (!fs.existsSync(configFile)) {
        let realConfig: Config | null = null;
        const sampleConfigPath = path.join(process.cwd(), "sample_config.json");

        if (fs.existsSync(sampleConfigPath)) {
          try {
            const raw = fs.readFileSync(sampleConfigPath, "utf-8");
            realConfig = JSON.parse(raw);
          } catch (e) {
            console.error("Failed to parse sample_config.json", e);
          }
        }

        if (!realConfig) {
          const realManager = new ConfigManager();
          if (realManager.hasConfigFile) {
            realConfig = realManager.get();
          }
        }

        if (realConfig) {
          fs.writeFileSync(configFile, JSON.stringify(realConfig));
        }
      }

      manager = new ConfigManager({
        cwd: tempConfigDir,
        overrides: {
          destDir: path.join(
            process.cwd(),
            "test-output",
            "Interface",
            "AddOns",
          ),
        },
      });
    } else {
      manager = new ConfigManager();
    }

    setConfigManager(manager);
    setAddonManager(new AddonManager(manager));
    if (manager.hasConfigFile) {
      const cfg = manager.get();
      setConfig(cfg);
      setTheme(cfg.theme);
    }
  }, [testMode, configManager, setTheme]);

  useEffect(() => {
    if (!configManager || config) return;

    if (!configManager.hasConfigFile) {
      setShowWizard(true);
      return;
    }

    const cfg = configManager.get();
    setConfig(cfg);
    setTheme(cfg.theme);

    if (initialLoad) {
      navigate("menu");
      setInitialLoad(false);
    }
  }, [configManager, config, initialLoad, navigate, setTheme]);

  const handleWizardComplete = () => {
    setShowWizard(false);
    if (configManager) {
      const cfg = configManager.get();
      setConfig(cfg);
    }

    // Check if addons were queued during wizard (ElvUI/TukUI or imports)
    const queue = useAppStore.getState().importQueue;
    if (queue.length > 0) {
      navigate("install");
    }
  };

  // Background auto-check for updates
  useEffect(() => {
    if (!addonManager || !config?.autoCheckEnabled) {
      setNextCheckTime(null);
      return;
    }

    const scheduleNextCheck = (delayMs: number) => {
      setNextCheckTime(Date.now() + delayMs);
    };

    const checkForUpdates = async () => {
      const addons = addonManager
        .getAllAddons()
        .filter((a) => a.type !== "manual");
      if (addons.length === 0) {
        scheduleNextCheck(config.autoCheckInterval);
        return;
      }

      const startTime = Date.now();
      const MIN_SPINNER_DURATION = 2000;

      setBackgroundChecking(true);
      progressBar.start(addons.length);
      let updateCount = 0;

      try {
        for (const addon of addons) {
          try {
            const result = await addonManager.checkUpdate(addon);
            if (result.updateAvailable) updateCount++;
          } catch {
            await progressBar.warn();
          }
          await progressBar.advance();
        }

        await progressBar.complete();

        if (updateCount > 0) {
          setPendingUpdates(updateCount);
          showToast(
            `${updateCount} update${updateCount > 1 ? "s" : ""} available`,
            5000,
          );
        }
      } finally {
        // Ensure spinner shows for at least MIN_SPINNER_DURATION
        const elapsed = Date.now() - startTime;
        const remaining = MIN_SPINNER_DURATION - elapsed;

        if (remaining > 0) {
          await new Promise((resolve) => setTimeout(resolve, remaining));
        }

        setBackgroundChecking(false);
        // Schedule next check
        scheduleNextCheck(config.autoCheckInterval);
      }
    };

    // Set initial next check time (5 seconds from now)
    scheduleNextCheck(5000);

    // Initial check after a short delay (5 seconds)
    const initialTimeout = setTimeout(checkForUpdates, 5000);

    // Recurring checks at the configured interval
    const interval = setInterval(checkForUpdates, config.autoCheckInterval);

    return () => {
      clearTimeout(initialTimeout);
      clearInterval(interval);
      setNextCheckTime(null);
    };
  }, [
    addonManager,
    config?.autoCheckEnabled,
    config?.autoCheckInterval,
    setPendingUpdates,
    setBackgroundChecking,
    setNextCheckTime,
    showToast,
    progressBar,
  ]);

  // Handle store-triggered onboarding (from ConfigScreen)
  const handleOnboardingComplete = () => {
    clearOnboarding();
    if (configManager) {
      const cfg = configManager.get();
      setConfig(cfg);
    }

    // Check if addons were queued during wizard (ElvUI/TukUI or imports)
    const queue = useAppStore.getState().importQueue;
    if (queue.length > 0) {
      navigate("install");
    }
  };

  if (showOnboarding && configManager) {
    return (
      <FirstRunWizard
        configManager={configManager}
        onComplete={handleOnboardingComplete}
      />
    );
  }

  if (showWizard && configManager) {
    return (
      <FirstRunWizard
        configManager={configManager}
        onComplete={handleWizardComplete}
      />
    );
  }

  if (!config || !configManager) {
    return <Text>Loading config...</Text>;
  }

  return (
    <Box
      flexDirection="column"
      padding={1}
      borderStyle="round"
      borderColor={theme.brand.split(".")[0]}
    >
      <Header dryRun={dryRun} isBusy={isBusy} testMode={testMode} />

      {activeScreen === "menu" && (
        <MainMenu
          config={config}
          configManager={configManager}
          initialSelection={lastMenuSelection}
          onSelect={(option) => {
            setLastMenuSelection(option);
            // @ts-expect-error: Screen type is broad
            navigate(option);
          }}
        />
      )}

      {activeScreen === "manage" && config && addonManager && (
        <ManageScreen
          config={config}
          addonManager={addonManager}
          force={force}
          dryRun={dryRun}
          onBack={() => {
            setConfig(addonManager.getConfig());
            navigate("menu");
          }}
        />
      )}

      {activeScreen === "install" && config && addonManager && (
        <InstallScreen
          config={config}
          addonManager={addonManager}
          onBack={() => {
            setConfig(addonManager.getConfig());
            navigate("menu");
          }}
        />
      )}

      {activeScreen === "config" && configManager && addonManager && (
        <ConfigScreen
          configManager={configManager}
          addonManager={addonManager}
          onBack={() => {
            setConfig(configManager.get());
            navigate("menu");
          }}
        />
      )}

      {activeScreen === "wagoSearch" && config && addonManager && (
        <WagoSearchScreen
          config={config}
          addonManager={addonManager}
          onBack={() => {
            setConfig(addonManager.getConfig());
            navigate("install");
          }}
        />
      )}
    </Box>
  );
};
