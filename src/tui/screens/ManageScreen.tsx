import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import Fuse from "fuse.js";
import { Box, Text, useApp, useInput } from "ink";
import Color from "ink-color-pipe";
import Spinner from "ink-spinner";
import TextInput from "ink-text-input";
import { useTerminalSize, VirtualList } from "ink-virtual-list";
import pLimit from "p-limit";
import type React from "react";
import { useCallback, useMemo, useState } from "react";
import { BackupManager } from "@/core/backup";
import type { Config } from "@/core/config";
import type { AddonManager, UpdateResult } from "@/core/manager";
import { ControlBar } from "@/tui/components/ControlBar";
import { HelpPanel } from "@/tui/components/HelpPanel";
import { type RepoStatus, RepositoryRow } from "@/tui/components/RepositoryRow";
import { ScreenTitle } from "@/tui/components/ScreenTitle";
import { MANAGE_SCREEN_RESERVED } from "@/tui/constants/layout";
import { useAddonManagerEvent } from "@/tui/hooks/useAddonManager";
import { useTheme } from "@/tui/hooks/useTheme";
import { useToast } from "@/tui/hooks/useToast";
import { useAppStore } from "@/tui/store/useAppStore";

interface ManageScreenProps {
  config: Config;
  addonManager: AddonManager;
  force?: boolean;
  dryRun?: boolean;
  onBack: () => void;
}

export const ManageScreen: React.FC<ManageScreenProps> = ({
  config,
  addonManager,
  force = false,
  onBack,
}) => {
  const { exit } = useApp();
  const queryClient = useQueryClient();
  const flashKey = useAppStore((state) => state.flashKey);
  const { theme } = useTheme();
  const { toast, showToast } = useToast();
  const { rows: terminalRows } = useTerminalSize();
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  const [refreshKey, setRefreshKey] = useState(0);
  const [showLibs, setShowLibs] = useState(config.showLibs);

  const [searchQuery, setSearchQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);
  const [sortConfig, setSortConfig] = useState<{
    key: "status" | "name" | "author" | "source";
    direction: "asc" | "desc";
  }>({ key: "name", direction: "asc" });

  const [updateProgress, setUpdateProgress] = useState<
    Record<string, RepoStatus>
  >({});

  // Confirmations
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<string[]>([]);
  const [confirmKind, setConfirmKind] = useState(false);
  const [pendingKindAddon, setPendingKindAddon] = useState<
    (typeof visibleAddons)[0] | null
  >(null);
  const [confirmBackup, setConfirmBackup] = useState(false);

  const getStatusPriority = useCallback(
    (folder: string) => {
      if (updateProgress[folder]) return 0;
      // biome-ignore lint/suspicious/noExplicitAny: React Query state type is dynamic
      const state = queryClient.getQueryState<any>(["addon", folder]);
      if (state?.fetchStatus === "fetching") return 0;
      if (state?.status === "error") return 2;
      if (state?.data?.updateAvailable) return 1;
      if (state?.data) return 4;
      return 3;
    },
    [updateProgress, queryClient],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: refreshKey is used to trigger re-fetching when the database state changes.
  const visibleAddons = useMemo(() => {
    const all = addonManager.getAllAddons();

    // Filter out owned folders (those in other addons' ownedFolders)
    const ownedFolders = new Set(all.flatMap((a) => a.ownedFolders));
    let filtered = all.filter((a) => !ownedFolders.has(a.folder));

    // Filter out libraries if showLibs is false
    if (!showLibs) {
      filtered = filtered.filter((a) => a.kind !== "library");
    }

    if (searchQuery.length > 0) {
      const fuse = new Fuse(filtered, {
        keys: ["name", "author"],
        threshold: 0.3,
      });
      filtered = fuse.search(searchQuery).map((r) => r.item);
    }

    filtered.sort((a, b) => {
      let res = 0;
      switch (sortConfig.key) {
        case "name":
          res = a.name.toLowerCase().localeCompare(b.name.toLowerCase());
          break;
        case "author":
          res = (a.author || "")
            .toLowerCase()
            .localeCompare((b.author || "").toLowerCase());
          break;
        case "source":
          res = a.type.localeCompare(b.type);
          if (res === 0) {
            res = (a.url || "")
              .toLowerCase()
              .localeCompare((b.url || "").toLowerCase());
          }
          break;
        case "status":
          res = getStatusPriority(a.folder) - getStatusPriority(b.folder);
          break;
      }
      return sortConfig.direction === "asc" ? res : -res;
    });

    // Build result with owned folders as children when showLibs is true
    const result: {
      record: (typeof filtered)[0];
      isChild: boolean;
      isLastChild: boolean;
    }[] = [];

    for (const addon of filtered) {
      result.push({ record: addon, isChild: false, isLastChild: false });

      // Show owned folders as children (they don't have DB records, so create display entries)
      if (showLibs && addon.ownedFolders.length > 0) {
        addon.ownedFolders.forEach((folderName, i) => {
          // Create a display-only record for the owned folder
          const childRecord: (typeof filtered)[0] = {
            id: undefined,
            name: folderName,
            folder: folderName,
            ownedFolders: [],
            kind: "addon",
            kindOverride: false,
            flavor: addon.flavor,
            version: addon.version,
            git_commit: addon.git_commit,
            author: addon.author,
            interface: addon.interface,
            url: addon.url,
            type: addon.type,
            requiredDeps: [],
            optionalDeps: [],
            embeddedLibs: [],
            install_date: addon.install_date,
            last_updated: addon.last_updated,
            last_checked: null,
            remote_version: null,
          };
          result.push({
            record: childRecord,
            isChild: true,
            isLastChild: i === addon.ownedFolders.length - 1,
          });
        });
      }
    }

    return result;
  }, [
    addonManager,
    refreshKey,
    showLibs,
    searchQuery,
    sortConfig,
    getStatusPriority,
  ]);

  const queries = useQueries({
    queries: visibleAddons.map((item) => ({
      queryKey: ["addon", item.record.folder],
      queryFn: async () => {
        // Pure passive read. No network requests.
        const freshAddon = addonManager.getAddon(item.record.folder);
        if (!freshAddon)
          return {
            updateAvailable: false,
            remoteVersion: "",
            error: "Addon not found in DB",
            checkedVersion: null,
            cached: true,
          };

        // We manually construct the status object based on DB state
        const config = addonManager.getConfig();
        const lastChecked = freshAddon.last_checked
          ? new Date(freshAddon.last_checked).getTime()
          : 0;
        const isStale = Date.now() - lastChecked > config.checkInterval;

        let updateAvailable = false;
        if (freshAddon.remote_version) {
          if (freshAddon.type === "github") {
            const localHash = freshAddon.git_commit || freshAddon.version;
            if (!localHash) {
              updateAvailable = true;
            } else {
              updateAvailable = localHash !== freshAddon.remote_version;
              if (
                freshAddon.remote_version.startsWith(localHash) ||
                localHash.startsWith(freshAddon.remote_version)
              ) {
                updateAvailable = false;
              }
            }
          } else {
            updateAvailable = freshAddon.version !== freshAddon.remote_version;
          }
        }

        return {
          updateAvailable,
          remoteVersion: freshAddon.remote_version || "",
          checkedVersion: freshAddon.version,
          cached: true, // Always cached in passive mode
          isStale,
        };
      },
      // Refetch when window focuses or network reconnects? Maybe.
      // But mainly we rely on invalidation from events.
      staleTime: Infinity,
    })),
  });

  // Track check start times outside of render to avoid setState during render
  // REMOVED: No longer tracking start times for passive mode.

  useAddonManagerEvent(
    addonManager,
    "addon:update-check:start",
    useCallback((folder) => {
      setUpdateProgress((prev) => ({ ...prev, [folder]: "checking" }));
    }, []),
  );

  useAddonManagerEvent(
    addonManager,
    "autocheck:progress",
    useCallback(
      (_, __, _addonName) => {
        // Invalidate query to refresh UI when background check updates an addon
        // Finding folder by name might be tricky if name != folder, but usually close.
        // Actually, we should probably emit folder in the event or invalidate all?
        // Invalidating all "addon" queries is safest and cheap since they are local DB reads.
        queryClient.invalidateQueries({ queryKey: ["addon"] });
      },
      [queryClient],
    ),
  );

  useAddonManagerEvent(
    addonManager,
    "addon:update-check:complete",
    useCallback(
      (folder) => {
        // Also invalidate specific folder on manual check complete
        setUpdateProgress((prev) => {
          const next = { ...prev };
          delete next[folder];
          return next;
        });
        queryClient.invalidateQueries({ queryKey: ["addon", folder] });
      },
      [queryClient],
    ),
  );

  useAddonManagerEvent(
    addonManager,
    "autocheck:progress",
    useCallback(
      (_, __, _addonName) => {
        // Invalidate query to refresh UI when background check updates an addon
        // Finding folder by name might be tricky if name != folder, but usually close.
        // Actually, we should probably emit folder in the event or invalidate all?
        // Invalidating all "addon" queries is safest and cheap since they are local DB reads.
        queryClient.invalidateQueries({ queryKey: ["addon"] });
      },
      [queryClient],
    ),
  );

  useAddonManagerEvent(
    addonManager,
    "addon:update-check:complete",
    useCallback(
      (folder) => {
        // Also invalidate specific folder on manual check complete
        setUpdateProgress((prev) => {
          const next = { ...prev };
          delete next[folder];
          return next;
        });
        queryClient.invalidateQueries({ queryKey: ["addon", folder] });
      },
      [queryClient],
    ),
  );

  useAddonManagerEvent(
    addonManager,
    "addon:install:downloading",
    useCallback((folder) => {
      setUpdateProgress((prev) => ({ ...prev, [folder]: "downloading" }));
    }, []),
  );

  useAddonManagerEvent(
    addonManager,
    "addon:install:extracting",
    useCallback((folder) => {
      setUpdateProgress((prev) => ({ ...prev, [folder]: "extracting" }));
    }, []),
  );

  useAddonManagerEvent(
    addonManager,
    "addon:install:copying",
    useCallback((folder) => {
      setUpdateProgress((prev) => ({ ...prev, [folder]: "copying" }));
    }, []),
  );

  useAddonManagerEvent(
    addonManager,
    "addon:install:complete",
    useCallback((folder) => {
      setUpdateProgress((prev) => {
        const next = { ...prev };
        delete next[folder];
        return next;
      });
    }, []),
  );

  const updateMutation = useMutation({
    mutationFn: async ({ folder }: { folder: string }) => {
      const os = await import("node:os");
      const path = await import("node:path");
      const fs = await import("node:fs/promises");

      const tempDir = path.join(os.tmpdir(), "lemonup-manage-single");
      await fs.mkdir(tempDir, { recursive: true });

      const addon = addonManager.getAddon(folder);
      if (!addon) throw new Error("Addon not found");

      try {
        const result = await addonManager.updateAddon(addon, force);
        return result;
      } finally {
        await fs.rm(tempDir, { recursive: true, force: true }).catch(() => {});
        setUpdateProgress((prev) => {
          const next = { ...prev };
          delete next[addon.folder];
          return next;
        });
      }
    },
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({
        queryKey: ["addon", variables.folder],
      });
    },
  });

  // Helpers
  const runUpdates = async (foldersToUpdate: string[]) => {
    if (foldersToUpdate.length === 0) return;

    const now = Date.now();

    // Filter out addons that were recently checked with no update available
    const foldersNeedingUpdate = foldersToUpdate.filter((folder) => {
      const queryKey = ["addon", folder];
      const state = queryClient.getQueryState<{ updateAvailable: boolean }>(
        queryKey,
      );
      const dataUpdatedAt = state?.dataUpdatedAt ?? 0;

      // If recently checked and no update available, skip
      if (dataUpdatedAt > 0 && now - dataUpdatedAt < config.checkInterval) {
        if (state?.data?.updateAvailable === false) {
          return false;
        }
      }
      return true;
    });

    if (foldersNeedingUpdate.length === 0) {
      showToast("Skipped (Recently Checked)", 2000);
      return;
    }

    showToast(
      `Updating ${foldersNeedingUpdate.length} addon${foldersNeedingUpdate.length > 1 ? "s" : ""}...`,
      0,
    );

    // Enforce concurrency limit
    const limit = pLimit(config.maxConcurrent);

    const promises = foldersNeedingUpdate.map((folder) => {
      return limit(async () => {
        await updateMutation.mutateAsync({ folder });
      });
    });

    await Promise.all(promises);

    showToast("Job's Done");
  };

  const runChecks = async (foldersToCheck: string[]) => {
    if (!foldersToCheck.length) return;

    const limit = pLimit(config.maxConcurrent);

    const results = await Promise.all(
      foldersToCheck.map((folder) => {
        return limit(async () => {
          const idx = visibleAddons.findIndex(
            (r) => r.record.folder === folder,
          );

          const queryKey = ["addon", folder];

          const state = queryClient.getQueryState(queryKey);
          const now = Date.now();
          const dataUpdatedAt = state?.dataUpdatedAt ?? 0;

          // If data exists and is younger than checkInterval, skip
          if (dataUpdatedAt > 0 && now - dataUpdatedAt < config.checkInterval) {
            return false;
          }

          if (idx !== -1 && queries[idx]) {
            await queries[idx].refetch();

            return true;
          }

          return false;
        });
      }),
    );

    const checkedCount = results.filter(Boolean).length;

    if (checkedCount === 0 && foldersToCheck.length > 0) {
      showToast("Skipped (Recently Checked)", 2000);
    }
  };

  const deleteMutation = useMutation({
    mutationFn: async ({ folder }: { folder: string }) => {
      await addonManager.removeAddon(folder);
    },
    onSuccess: (_, variables) => {
      if (selectedIds.has(variables.folder)) {
        setSelectedIds((prev) => {
          const next = new Set(prev);
          next.delete(variables.folder);
          return next;
        });
      }
    },
  });

  const runDeletes = async (foldersToDelete: string[]) => {
    showToast("Deleting...", 0);
    for (const folder of foldersToDelete) {
      await deleteMutation.mutateAsync({ folder });
    }
    setRefreshKey((prev) => prev + 1); // Trigger re-render to refresh addon list
    showToast(`Deleted ${foldersToDelete.length} addons.`);
    setConfirmDelete(false);
  };

  const [showMenu, setShowMenu] = useState(false);

  useInput((input, key) => {
    if (isSearching) {
      if (key.escape) {
        setIsSearching(false);
        setSearchQuery("");
      }
      if (key.return) {
        setIsSearching(false);
      }
      return;
    }

    if (confirmDelete) {
      if (input === "y" || key.return) {
        flashKey("y");
        runDeletes(pendingDelete);
      } else if (input === "n" || key.escape) {
        flashKey("n");
        setConfirmDelete(false);
        setPendingDelete([]);
      }
      return;
    }

    if (confirmKind) {
      if (input === "y" || key.return) {
        flashKey("y");
        if (pendingKindAddon) {
          const addon = pendingKindAddon.record;
          const newKind = addon.kind === "library" ? "addon" : "library";
          addonManager.updateAddonMetadata(addon.folder, {
            kind: newKind,
            kindOverride: true,
          });
          setRefreshKey((prev) => prev + 1);
          showToast(`${addon.name} marked as ${newKind}`);
        }
        setConfirmKind(false);
        setPendingKindAddon(null);
      } else if (input === "n" || key.escape) {
        flashKey("n");
        setConfirmKind(false);
        setPendingKindAddon(null);
      }
      return;
    }

    if (confirmBackup) {
      if (input === "y" || key.return) {
        flashKey("y");
        const runBackup = async () => {
          showToast("Backing up WTF...", 0);
          try {
            const result = await BackupManager.backupWTF(config.destDir);

            if (result === null) {
              showToast("Backup Failed: WTF folder not found");
            } else if (result === "skipped-recent") {
              showToast("Backup Skipped (Too Recent)");
            } else {
              // Result is the path to the zip file
              await BackupManager.cleanupBackups(
                config.destDir,
                config.backupRetention,
              );
              showToast("Backup Complete!");
            }
          } catch (error) {
            showToast(`Backup Failed: ${(error as Error).message}`);
          }
        };
        runBackup();
        setConfirmBackup(false);
      } else if (input === "n" || key.escape) {
        flashKey("n");
        setConfirmBackup(false);
      }
      return;
    }

    if (key.escape) {
      if (showMenu) {
        setShowMenu(false);
        return;
      }
      // Clear filter first if active, then go back
      if (searchQuery.length > 0) {
        setSearchQuery("");
        return;
      }
      onBack();
      return;
    }

    if (input === "/") {
      setIsSearching(true);
      return;
    }

    if (["1", "2", "3", "4"].includes(input)) {
      const map: Record<string, "status" | "name" | "author" | "source"> = {
        "1": "status",
        "2": "name",
        "3": "author",
        "4": "source",
      };
      const k = map[input];
      if (k) {
        setSortConfig((prev) => ({
          key: k,
          direction:
            prev.key === k && prev.direction === "asc" ? "desc" : "asc",
        }));
      }
      return;
    }

    if (input === "m") {
      flashKey("m");
      setShowMenu((prev) => !prev);
      return;
    }

    if (input === "l") {
      flashKey("l");
      const newVal = !showLibs;
      setShowLibs(newVal);
      addonManager.setConfigValue("showLibs", newVal);
      setSelectedIndex(0);
      return;
    }

    if (input === "q") {
      exit();
      return;
    }

    if (showMenu && ["k", "j", " ", "u", "c", "d"].includes(input)) {
      setShowMenu(false);
    }
    if (showMenu && (key.upArrow || key.downArrow)) {
      setShowMenu(false);
    }

    const getNextIndex = (current: number, direction: 1 | -1) => {
      let next = current + direction;
      while (next >= 0 && next < visibleAddons.length) {
        const item = visibleAddons[next];
        if (item && !item.isChild) {
          return next;
        }
        next += direction;
      }
      return current;
    };

    if (key.upArrow || input === "k") {
      flashKey("up");
      setSelectedIndex((prev) => getNextIndex(prev, -1));
    }

    if (key.downArrow || input === "j") {
      flashKey("down");
      setSelectedIndex((prev) => getNextIndex(prev, 1));
    }

    if (input === " ") {
      flashKey(" ");
      const currentItem = visibleAddons[selectedIndex];
      if (currentItem && !currentItem.isChild) {
        // Only allow selecting parent addons
        setSelectedIds((prev) => {
          const next = new Set(prev);
          if (next.has(currentItem.record.folder)) {
            next.delete(currentItem.record.folder);
          } else {
            next.add(currentItem.record.folder);
          }
          return next;
        });
      }
    }

    if (input === "u") {
      flashKey("u");
      if (selectedIds.size > 0) {
        runUpdates(Array.from(selectedIds));
      } else {
        const currentItem = visibleAddons[selectedIndex];
        if (currentItem && !currentItem.isChild) {
          runUpdates([currentItem.record.folder]);
        }
      }
    }

    if (input === "U") {
      flashKey("U");
      const allFolders = visibleAddons
        .filter((a) => !a.isChild)
        .map((a) => a.record.folder);
      runUpdates(allFolders);
    }

    if (input === "c") {
      flashKey("c");
      if (selectedIds.size > 0) {
        runChecks(Array.from(selectedIds));
      } else {
        const currentItem = visibleAddons[selectedIndex];
        if (currentItem && !currentItem.isChild) {
          runChecks([currentItem.record.folder]);
        }
      }
    }

    if (input === "d" || key.delete) {
      const targets =
        selectedIds.size > 0
          ? Array.from(selectedIds)
          : visibleAddons[selectedIndex] &&
              !visibleAddons[selectedIndex].isChild
            ? [visibleAddons[selectedIndex].record.folder]
            : [];
      if (targets.length > 0) {
        setPendingDelete(targets);
        setConfirmDelete(true);
      }
    }

    if (input === "t") {
      flashKey("t");
      const currentItem = visibleAddons[selectedIndex];
      if (currentItem && !currentItem.isChild) {
        setPendingKindAddon(currentItem);
        setConfirmKind(true);
      }
    }

    if (input === "b") {
      flashKey("b");
      if (showMenu) setShowMenu(false);
      setConfirmBackup(true);
    }
  });

  return (
    <Box flexDirection="column" gap={1} height="100%" width="100%">
      <ScreenTitle title="Manage Addons">
        {isSearching ? (
          <Box>
            {searchQuery.length > 0 && <Text color="cyan">Search: </Text>}
            <TextInput
              value={searchQuery}
              onChange={setSearchQuery}
              placeholder="Search addons by name or author (esc to cancel):"
            />
          </Box>
        ) : searchQuery.length > 0 ? (
          <Box>
            <Text color="cyan">Filtered: "{searchQuery}"</Text>
            <Text color="gray"> [/] edit [esc] clear</Text>
          </Box>
        ) : (
          <Text color="cyan">[/] search</Text>
        )}
      </ScreenTitle>

      <Box
        borderStyle="single"
        borderColor="gray"
        paddingX={1}
        marginTop={1}
        marginBottom={0}
        width="100%"
      >
        <Box width={3} flexShrink={0}>
          <Text> </Text>
        </Box>
        <Box width={22} flexShrink={0}>
          <Text bold>
            Status{" "}
            {sortConfig.key === "status"
              ? sortConfig.direction === "asc"
                ? "▲"
                : "▼"
              : ""}
          </Text>
        </Box>
        <Box flexGrow={2} flexShrink={1} minWidth={15} flexBasis="20%">
          <Text bold>
            Name{" "}
            {sortConfig.key === "name"
              ? sortConfig.direction === "asc"
                ? "▲"
                : "▼"
              : ""}
          </Text>
        </Box>
        <Box flexGrow={1} flexShrink={1} minWidth={10} flexBasis="15%">
          <Text bold>
            Author{" "}
            {sortConfig.key === "author"
              ? sortConfig.direction === "asc"
                ? "▲"
                : "▼"
              : ""}
          </Text>
        </Box>
        <Box width={8} flexShrink={0}>
          <Text bold>
            Source{" "}
            {sortConfig.key === "source"
              ? sortConfig.direction === "asc"
                ? "▲"
                : "▼"
              : ""}
          </Text>
        </Box>
      </Box>

      {visibleAddons.length === 0 ? (
        searchQuery.length > 0 ? (
          <Box
            flexDirection="column"
            alignItems="center"
            justifyContent="center"
            paddingY={5}
            width="100%"
          >
            <Text color="yellow" bold italic>
              "By the Light! No results found for '{searchQuery}'"
            </Text>
            <Box marginTop={1}>
              <Text color="gray">
                Try adjusting your search criteria or clear the search.
              </Text>
            </Box>
          </Box>
        ) : (
          <Box
            flexDirection="column"
            alignItems="center"
            justifyContent="center"
            paddingY={5}
            width="100%"
          >
            <Text color="yellow" bold italic>
              "Lok-tar ogar! ...Wait, where is everyone?"
            </Text>
            <Box marginTop={1}>
              <Text color="gray">
                Your inventory is empty. No addons found in your bags.
              </Text>
            </Box>
            <Box marginTop={1}>
              <Text color="cyan">
                Visit the 'Install Addon' section to start your collection!
              </Text>
            </Box>
          </Box>
        )
      ) : (
        <VirtualList
          items={visibleAddons}
          selectedIndex={selectedIndex}
          keyExtractor={(item) => item.record.folder}
          height={terminalRows - MANAGE_SCREEN_RESERVED}
          renderItem={({ item, index: idx, isSelected }) => {
            const addon = item.record;
            const isChecked = selectedIds.has(addon.folder);

            const query = queries[idx];
            if (!query) return null;
            const { data } = query;
            const { isLoading, isFetching, error } = query;

            const isUpdating =
              updateMutation.isPending &&
              updateMutation.variables?.folder === addon.folder;

            let status: RepoStatus = "idle";
            let result: UpdateResult | undefined;

            // Synchronous check for cached status to avoid flicker
            // const cachedStatus = addonManager.getCachedUpdateStatus(addon);

            if (isUpdating) {
              status = updateProgress[addon.folder] || "checking";
            } else if (isLoading || isFetching) {
              // Should be fast since it's local DB read
              status = "checking";
            } else {
              if (error) status = "error";
              else if (data) status = "done";
            }

            if (status === "done" && data) {
              if (data.updateAvailable) {
                const versionDisplay = data.remoteVersion
                  ? data.remoteVersion.length > 10
                    ? data.remoteVersion.substring(0, 7)
                    : data.remoteVersion
                  : "";
                result = {
                  repoName: addon.name,
                  success: true,
                  updated: true,
                  message: versionDisplay
                    ? `Update: ${versionDisplay}`
                    : "Update Available",
                };
              } else {
                result = {
                  repoName: addon.name,
                  success: true,
                  updated: false,
                  message: "Up to date",
                };
              }
            }

            return (
              <RepositoryRow
                repo={addon}
                status={status}
                result={result}
                nerdFonts={config.nerdFonts}
                isSelected={isSelected}
                isChecked={isChecked}
                isChild={item.isChild}
                isLastChild={item.isLastChild}
                showLibs={showLibs}
              />
            );
          }}
        />
      )}

      <ControlBar
        message={
          confirmDelete ? (
            <Text color="red" bold>
              Confirm: Delete{" "}
              {pendingDelete.length > 1
                ? `${pendingDelete.length} addons`
                : pendingDelete[0]}
              ?
            </Text>
          ) : confirmKind ? (
            <Text color="cyan" bold>
              Confirm: Toggle type for {pendingKindAddon?.record.name}?
            </Text>
          ) : confirmBackup ? (
            <Text color="cyan" bold>
              Confirm: Backup WTF folder?
            </Text>
          ) : toast?.message ? (
            toast.message.includes("Done") ||
            toast.message.includes("Complete") ||
            toast.message.includes("Deleted") ? (
              <Text color="green">✔ {toast.message}</Text>
            ) : toast.message.includes("Skipped") ? (
              <Text color="cyan">ℹ {toast.message}</Text>
            ) : (
              <Text color="yellow">
                {/* @ts-expect-error: Spinner types mismatch */}
                <Spinner type="dots" /> {toast.message}
              </Text>
            )
          ) : (
            <Box>
              <Color
                styles={selectedIds.size > 0 ? theme.checked : theme.muted}
              >
                <Text bold={selectedIds.size > 0}>
                  Selected: {selectedIds.size} /{" "}
                  {visibleAddons.filter((a) => !a.isChild).length}
                </Text>
              </Color>
              {showLibs && (
                <Color styles={theme.muted}>
                  <Text>
                    {" "}
                    [Libs:{" "}
                    <Color styles={theme.success}>
                      <Text>Visible</Text>
                    </Color>
                    ]
                  </Text>
                </Color>
              )}
            </Box>
          )
        }
        controls={
          confirmDelete || confirmKind || confirmBackup
            ? [
                { key: "y", label: "confirm" },
                { key: "n", label: "cancel" },
              ]
            : [
                { key: "↑/↓", label: "nav" },
                { key: "space", label: "select" },
                { key: "u", label: "update" },
                { key: "U", label: "update all" },
                { key: "c", label: "check" },
                { key: "1-4", label: "sort" },
                { key: "m", label: "menu" },
              ]
        }
      />

      <HelpPanel
        expanded={showMenu}
        shortcuts={[
          { key: "↑/↓", label: "Navigate" },
          { key: "/", label: "Search/Filter" },
          { key: "Space", label: "Select/Deselect" },
          { key: "u", label: "Update Selected" },
          { key: "U", label: "Update All" },
          { key: "c", label: "Check Updates" },
          { key: "l", label: "Toggle Libs" },
          { key: "t", label: "Toggle Kind" },
          { key: "d", label: "Delete Selected" },
          { key: "b", label: "Backup WTF" },
          { key: "q", label: "Quit Application" },
          { key: "Esc", label: "Go Back / Close" },
        ]}
      />
    </Box>
  );
};
