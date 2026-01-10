import { Box, Text, useInput } from "ink";
import Color from "ink-color-pipe";
import Spinner from "ink-spinner";
import TextInput from "ink-text-input";
import { useTerminalSize, VirtualList } from "ink-virtual-list";
import type React from "react";
import { useCallback, useMemo, useState } from "react";
import type { Config } from "@/core/config";
import type { AddonManager } from "@/core/manager";
import type { WagoAddonSummary, WagoStability } from "@/core/wago";
import { ControlBar } from "@/tui/components/ControlBar";
import { ScreenTitle } from "@/tui/components/ScreenTitle";
import { WagoResultRow } from "@/tui/components/WagoResultRow";
import { WAGO_SEARCH_RESERVED } from "@/tui/constants/layout";
import { useTheme } from "@/tui/hooks/useTheme";
import { useWagoSearch } from "@/tui/hooks/useWagoSearch";
import { useAppStore } from "@/tui/store/useAppStore";

interface WagoSearchScreenProps {
  config: Config;
  addonManager: AddonManager;
  onBack: () => void;
}

type Mode =
  | "search"
  | "filters"
  | "details"
  | "confirm-reinstall"
  | "installing"
  | "result";

const GAME_VERSIONS = [
  "retail",
  "classic",
  "cata",
  "wotlk",
  "bc",
  "mop",
] as const;
const STABILITIES = ["stable", "beta", "alpha"] as const;

function formatDownloads(count: number | undefined): string {
  if (!count) return "-";
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${Math.floor(count / 1_000)}K`;
  return count.toString();
}

export const WagoSearchScreen: React.FC<WagoSearchScreenProps> = ({
  config,
  addonManager,
  onBack,
}) => {
  const { theme } = useTheme();
  const flashKey = useAppStore((state) => state.flashKey);
  const { rows: terminalRows } = useTerminalSize();

  const [mode, setMode] = useState<Mode>("search");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [showFilters, setShowFilters] = useState(false);
  const [isSearching, setIsSearching] = useState(true);
  const [sortConfig, setSortConfig] = useState<{
    key: "name" | "author" | "downloads";
    direction: "asc" | "desc";
  }>({ key: "downloads", direction: "desc" });

  const [filterFocus, setFilterFocus] = useState<"gameVersion" | "stability">(
    "gameVersion",
  );
  const [selectedAddon, setSelectedAddon] = useState<WagoAddonSummary | null>(
    null,
  );
  const [installStatus, setInstallStatus] = useState<
    "idle" | "installing" | "success" | "error"
  >("idle");
  const [installError, setInstallError] = useState<string | null>(null);
  const [installedName, setInstalledName] = useState<string>("");
  const [pendingInstall, setPendingInstall] = useState<{
    addon: WagoAddonSummary;
    stability: WagoStability;
  } | null>(null);

  const showToast = useAppStore((state) => state.showToast);

  const {
    query,
    setQuery,
    results,
    isLoading,
    error,
    gameVersion,
    setGameVersion,
    stability,
    setStability,
  } = useWagoSearch({ apiKey: config.wagoApiKey });

  const sortedResults = useMemo(() => {
    return [...results].sort((a, b) => {
      let res = 0;
      switch (sortConfig.key) {
        case "name":
          res = a.display_name.localeCompare(b.display_name);
          break;
        case "author": {
          const authA = a.owner ?? a.authors?.[0] ?? "";
          const authB = b.owner ?? b.authors?.[0] ?? "";
          res = authA.localeCompare(authB);
          break;
        }
        case "downloads":
          res = (a.download_count ?? 0) - (b.download_count ?? 0);
          break;
      }
      return sortConfig.direction === "asc" ? res : -res;
    });
  }, [results, sortConfig]);

  const listHeight = Math.max(
    1,
    terminalRows - WAGO_SEARCH_RESERVED - (showFilters ? 2 : 0),
  );

  const hasApiKey = Boolean(config.wagoApiKey);

  const isInstalled = useCallback(
    (addon: WagoAddonSummary) => {
      // Check by Wago URL ID
      const wagoUrl = `https://addons.wago.io/addons/${addon.id}`;
      if (addonManager.isAlreadyInstalled(wagoUrl)) return true;

      // Fallback: Check by Name (Fuzzy/Exact)
      const installed = addonManager.getAllAddons();
      const normalize = (s: string) => s.toLowerCase().replace(/\s+/g, "");
      const target = normalize(addon.display_name);

      return installed.some(
        (a) => normalize(a.name) === target || normalize(a.folder) === target,
      );
    },
    [addonManager],
  );

  const doInstall = useCallback(
    async (addon: WagoAddonSummary, installStability: WagoStability) => {
      setMode("installing");
      setInstallStatus("installing");
      setInstallError(null);
      setInstalledName(addon.display_name);
      setPendingInstall(null);

      try {
        const result = await addonManager.installWago(
          addon.id,
          installStability,
        );

        if (result.success) {
          setInstallStatus("success");
          showToast(`Installed ${addon.display_name}!`, 3000);
        } else {
          setInstallStatus("error");
          setInstallError(result.error || "Install failed");
        }
      } catch (e) {
        setInstallStatus("error");
        setInstallError(e instanceof Error ? e.message : "Install failed");
      }

      setMode("result");
    },
    [addonManager, showToast],
  );

  const handleInstall = useCallback(
    (addon: WagoAddonSummary, installStability: WagoStability) => {
      // Check if addon is already installed
      if (isInstalled(addon)) {
        setPendingInstall({ addon, stability: installStability });
        setMode("confirm-reinstall");
        return;
      }

      doInstall(addon, installStability);
    },
    [doInstall, isInstalled],
  );

  useInput((input, key) => {
    // Handle no API key mode - only escape works
    if (!hasApiKey) {
      if (key.escape) {
        flashKey("esc");
        onBack();
      }
      return;
    }

    // Handle installing mode - no input
    if (mode === "installing") {
      return;
    }

    // Handle result mode
    if (mode === "result") {
      if (key.return || key.escape) {
        flashKey("enter");
        setMode("search");
        setSelectedAddon(null);
        setInstallStatus("idle");
        setInstallError(null);
      }
      return;
    }

    // Handle confirm-reinstall mode
    if (mode === "confirm-reinstall") {
      if (input.toLowerCase() === "y" && pendingInstall) {
        flashKey("enter");
        doInstall(pendingInstall.addon, pendingInstall.stability);
        return;
      }
      if (input.toLowerCase() === "n" || key.escape) {
        flashKey("esc");
        setPendingInstall(null);
        setMode("search");
        return;
      }
      return;
    }

    // Handle filter mode
    if (showFilters) {
      if (key.escape || input === "f") {
        flashKey("f");
        setShowFilters(false);
        return;
      }
      if (key.tab) {
        setFilterFocus((f) =>
          f === "gameVersion" ? "stability" : "gameVersion",
        );
        return;
      }
      if (key.leftArrow || input === "h") {
        flashKey("h");
        if (filterFocus === "gameVersion") {
          const idx = GAME_VERSIONS.indexOf(gameVersion);
          const newIdx = idx > 0 ? idx - 1 : GAME_VERSIONS.length - 1;
          const newVersion = GAME_VERSIONS[newIdx];
          if (newVersion) setGameVersion(newVersion);
        } else {
          const idx = STABILITIES.indexOf(stability);
          const newIdx = idx > 0 ? idx - 1 : STABILITIES.length - 1;
          const newStability = STABILITIES[newIdx];
          if (newStability) setStability(newStability);
        }
        return;
      }
      if (key.rightArrow || input === "l") {
        flashKey("l");
        if (filterFocus === "gameVersion") {
          const idx = GAME_VERSIONS.indexOf(gameVersion);
          const newIdx = (idx + 1) % GAME_VERSIONS.length;
          const newVersion = GAME_VERSIONS[newIdx];
          if (newVersion) setGameVersion(newVersion);
        } else {
          const idx = STABILITIES.indexOf(stability);
          const newIdx = (idx + 1) % STABILITIES.length;
          const newStability = STABILITIES[newIdx];
          if (newStability) setStability(newStability);
        }
        return;
      }
      return;
    }

    // Handle details mode
    if (mode === "details") {
      if (key.escape) {
        flashKey("esc");
        setMode("search");
        setSelectedAddon(null);
        return;
      }
      if ((key.return || input === "i") && selectedAddon) {
        flashKey("enter");
        handleInstall(selectedAddon, stability);
        return;
      }
      return;
    }

    // Handle search mode
    if (mode === "search") {
      if (isSearching) {
        // Edit Mode
        if (key.escape || key.return) {
          flashKey(key.escape ? "esc" : "enter");
          setIsSearching(false);
          return;
        }
        if (key.upArrow || key.downArrow) {
          flashKey(key.upArrow ? "up" : "down");
          setIsSearching(false);
          if (key.upArrow) setSelectedIndex((i) => Math.max(0, i - 1));
          if (key.downArrow)
            setSelectedIndex((i) => Math.min(sortedResults.length - 1, i + 1));
          return;
        }
        return;
      }

      // Nav Mode
      if (input === "/") {
        flashKey("/");
        setIsSearching(true);
        return;
      }

      if (key.escape) {
        flashKey("esc");
        onBack();
        return;
      }

      // Sorting
      if (["1", "2", "3"].includes(input)) {
        const map: Record<string, "name" | "author" | "downloads"> = {
          "1": "name",
          "2": "author",
          "3": "downloads",
        };
        const k = map[input];
        if (k) {
          flashKey(input);
          setSortConfig((prev) => ({
            key: k,
            direction:
              prev.key === k && prev.direction === "asc" ? "desc" : "asc",
          }));
        }
        return;
      }

      if (input === "f") {
        flashKey("f");
        setShowFilters(true);
        return;
      }

      if (key.upArrow || input === "k") {
        flashKey("k");
        setSelectedIndex((i) => Math.max(0, i - 1));
        return;
      }

      if (key.downArrow || input === "j") {
        flashKey("j");
        setSelectedIndex((i) => Math.min(sortedResults.length - 1, i + 1));
        return;
      }

      if (key.return && sortedResults[selectedIndex]) {
        flashKey("enter");
        const addon = sortedResults[selectedIndex];
        if (addon) {
          setSelectedAddon(addon);
          setMode("details");
        }
        return;
      }

      if (input === "i" && sortedResults[selectedIndex]) {
        flashKey("i");
        const addon = sortedResults[selectedIndex];
        if (addon) {
          handleInstall(addon, stability);
        }
        return;
      }
    }
  });

  // Reset selection when results change
  if (selectedIndex >= sortedResults.length && sortedResults.length > 0) {
    setSelectedIndex(0);
  }

  // Render no API key state
  if (!hasApiKey) {
    return (
      <Box flexDirection="column">
        <ScreenTitle title="Search Wago" />
        <Box
          flexDirection="column"
          alignItems="center"
          justifyContent="center"
          paddingY={2}
        >
          <Color styles={theme.statusWarning}>
            <Text bold>Wago API Key Required</Text>
          </Color>
          <Box marginTop={1}>
            <Text>
              To search Wago addons, configure an API key in Settings.
            </Text>
          </Box>
          <Box marginTop={1}>
            <Color styles={theme.muted}>
              <Text>Get one at: https://addons.wago.io/account/api</Text>
            </Color>
          </Box>
        </Box>
        <ControlBar controls={[{ key: "esc", label: "back" }]} />
      </Box>
    );
  }

  return (
    <Box flexDirection="column">
      <ScreenTitle title="Search Wago" />

      {/* Search Input */}
      <Box paddingX={1} marginBottom={1}>
        <Color styles={theme.muted}>
          <Text>Search: </Text>
        </Color>
        <TextInput
          value={query}
          onChange={setQuery}
          placeholder="type to search..."
          focus={mode === "search" && !showFilters && isSearching}
        />
        {isLoading && (
          <Box marginLeft={2}>
            <Color styles={theme.busy}>
              {/* @ts-expect-error: Spinner types mismatch */}
              <Spinner type="dots" />
            </Color>
          </Box>
        )}
        {!isSearching && !showFilters && mode === "search" && (
          <Box marginLeft={2}>
            <Color styles={theme.muted}>
              <Text>Press / to search</Text>
            </Color>
          </Box>
        )}
      </Box>

      {/* Filters (collapsible) */}
      {showFilters && (
        <Box paddingX={1} marginBottom={1} gap={4}>
          <Box>
            <Color
              styles={
                filterFocus === "gameVersion" ? theme.highlight : theme.muted
              }
            >
              <Text>Game: </Text>
              <Text bold>
                {"< "}
                {gameVersion}
                {" >"}
              </Text>
            </Color>
          </Box>
          <Box>
            <Color
              styles={
                filterFocus === "stability" ? theme.highlight : theme.muted
              }
            >
              <Text>Channel: </Text>
              <Text bold>
                {"< "}
                {stability}
                {" >"}
              </Text>
            </Color>
          </Box>
        </Box>
      )}

      {/* Error display */}
      {error && (
        <Box paddingX={1}>
          <Color styles={theme.statusError}>
            <Text>Error: {error}</Text>
          </Color>
        </Box>
      )}

      {/* Column headers */}
      {sortedResults.length > 0 && (
        <Box paddingX={1}>
          <Box width={2} />
          <Box width={30}>
            <Color styles={theme.muted}>
              <Text bold={sortConfig.key === "name"}>
                Name{" "}
                {sortConfig.key === "name"
                  ? sortConfig.direction === "asc"
                    ? "▲"
                    : "▼"
                  : ""}
              </Text>
            </Color>
          </Box>
          <Box width={14}>
            <Color styles={theme.muted}>
              <Text bold={sortConfig.key === "author"}>
                Author{" "}
                {sortConfig.key === "author"
                  ? sortConfig.direction === "asc"
                    ? "▲"
                    : "▼"
                  : ""}
              </Text>
            </Color>
          </Box>
          <Box width={8} justifyContent="flex-end">
            <Color styles={theme.muted}>
              <Text bold={sortConfig.key === "downloads"}>
                DLs{" "}
                {sortConfig.key === "downloads"
                  ? sortConfig.direction === "asc"
                    ? "▲"
                    : "▼"
                  : ""}
              </Text>
            </Color>
          </Box>
          <Box marginLeft={2}>
            <Color styles={theme.muted}>
              <Text>Summary</Text>
            </Color>
          </Box>
        </Box>
      )}

      {/* Results list */}
      {sortedResults.length > 0 ? (
        <VirtualList
          items={sortedResults}
          selectedIndex={selectedIndex}
          height={listHeight}
          keyExtractor={(item) => item.id}
          renderItem={({ item, index }) => (
            <WagoResultRow
              addon={item}
              isSelected={index === selectedIndex}
              isInstalled={isInstalled(item)}
            />
          )}
        />
      ) : query && !isLoading && !error ? (
        <Box flexDirection="column" alignItems="center" paddingY={2}>
          <Color styles={theme.muted}>
            <Text>No addons found matching "{query}"</Text>
          </Color>
        </Box>
      ) : !query ? (
        <Box flexDirection="column" alignItems="center" paddingY={2}>
          <Color styles={theme.muted}>
            <Text>Start typing to search Wago addons...</Text>
          </Color>
        </Box>
      ) : null}

      {/* Details panel */}
      {mode === "details" && selectedAddon && (
        <Box
          flexDirection="column"
          borderStyle="round"
          borderColor="cyan"
          padding={1}
          marginX={1}
        >
          <Box>
            <Color styles={theme.highlight}>
              <Text bold>{selectedAddon.display_name}</Text>
            </Color>
            <Text> by </Text>
            <Text>
              {selectedAddon.owner ?? selectedAddon.authors?.[0] ?? "Unknown"}
            </Text>
          </Box>
          <Box marginTop={1}>
            <Text>{selectedAddon.summary}</Text>
          </Box>
          <Box marginTop={1} gap={4}>
            <Text>
              Downloads: {formatDownloads(selectedAddon.download_count)}
            </Text>
            <Text>Version: {selectedAddon.releases?.stable?.label ?? "-"}</Text>
          </Box>
          <Box marginTop={1}>
            <Color styles={theme.muted}>
              <Text>[Enter] Install [Esc] Back</Text>
            </Color>
          </Box>
        </Box>
      )}

      {/* Confirm reinstall */}
      {mode === "confirm-reinstall" && pendingInstall && (
        <Box
          flexDirection="column"
          borderColor="red"
          borderStyle="round"
          padding={1}
          marginX={1}
        >
          <Color styles={theme.statusWarning}>
            <Text bold>Addon Already Installed</Text>
          </Color>
          <Box marginTop={1}>
            <Text>
              "{pendingInstall.addon.display_name}" appears to be already
              installed.
            </Text>
          </Box>
          <Box marginTop={1}>
            <Text>Do you want to reinstall and overwrite it? (y/N)</Text>
          </Box>
        </Box>
      )}

      {/* Installing state */}
      {mode === "installing" && (
        <Box flexDirection="column" alignItems="center" paddingY={2}>
          <Color styles={theme.busy}>
            <Text>
              {/* @ts-expect-error: Spinner types mismatch */}
              <Spinner type="dots" /> Installing {installedName}...
            </Text>
          </Color>
        </Box>
      )}

      {/* Result state */}
      {mode === "result" && (
        <Box flexDirection="column" paddingX={1} paddingY={1}>
          {installStatus === "success" ? (
            <Color styles={theme.success}>
              <Text bold>Installed {installedName}!</Text>
            </Color>
          ) : (
            <Box flexDirection="column">
              <Color styles={theme.error}>
                <Text bold>Installation failed</Text>
              </Color>
              {installError && (
                <Box marginTop={1}>
                  <Color styles={theme.muted}>
                    <Text>{installError}</Text>
                  </Color>
                </Box>
              )}
            </Box>
          )}
          <Box marginTop={1}>
            <Color styles={theme.muted}>
              <Text>Press Enter to continue</Text>
            </Color>
          </Box>
        </Box>
      )}

      <ControlBar
        controls={
          mode === "installing"
            ? []
            : mode === "result"
              ? [{ key: "enter", label: "continue" }]
              : mode === "confirm-reinstall"
                ? [
                    { key: "y", label: "reinstall" },
                    { key: "n", label: "cancel" },
                  ]
                : showFilters
                  ? [
                      { key: "h/l", label: "change" },
                      { key: "tab", label: "switch" },
                      { key: "f", label: "close filters" },
                    ]
                  : mode === "details"
                    ? [
                        { key: "enter", label: "install" },
                        { key: "esc", label: "back" },
                      ]
                    : isSearching
                      ? [
                          { key: "esc", label: "stop editing" },
                          { key: "enter", label: "stop editing" },
                          { key: "↑/↓", label: "nav" },
                        ]
                      : [
                          { key: "/", label: "search" },
                          { key: "j/k", label: "nav" },
                          { key: "enter", label: "details" },
                          { key: "i", label: "install" },
                          { key: "f", label: "filters" },
                          { key: "1-3", label: "sort" },
                          { key: "esc", label: "back" },
                        ]
        }
      />
    </Box>
  );
};
