import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import Fuse from "fuse.js";
import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import TextInput from "ink-text-input";
import pLimit from "p-limit";
import type React from "react";
import { useCallback, useMemo, useState } from "react";
import { BackupManager } from "@/core/backup";
import type { Config } from "@/core/config";
import type { AddonManager, UpdateResult } from "@/core/manager";
import { ControlBar } from "@/tui/components/ControlBar";
import { HelpPanel } from "@/tui/components/HelpPanel";
import { type RepoStatus, RepositoryRow } from "@/tui/components/RepositoryRow";
import { useAddonManagerEvent } from "@/tui/hooks/useAddonManager";
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
	const { toast, showToast } = useToast();
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

	const [refreshKey, setRefreshKey] = useState(0);
	const [showLibs, setShowLibs] = useState(false);

	const [searchQuery, setSearchQuery] = useState("");
	const [isSearching, setIsSearching] = useState(false);
	const [sortConfig, setSortConfig] = useState<{
		key: "status" | "name" | "author" | "source";
		direction: "asc" | "desc";
	}>({ key: "name", direction: "asc" });

	const [updateProgress, setUpdateProgress] = useState<
		Record<string, RepoStatus>
	>({});

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
				const freshAddon = addonManager.getAddon(item.record.folder);
				if (!freshAddon)
					return {
						updateAvailable: false,
						remoteVersion: "",
						error: "Addon not found in DB",
						checkedVersion: null,
					};

				const res = await addonManager.checkUpdate(freshAddon);
				return { ...res, checkedVersion: freshAddon.version };
			},
			staleTime: config.checkInterval,
		})),
	});

	useAddonManagerEvent(
		addonManager,
		"addon:update-check:start",
		useCallback((folder) => {
			setUpdateProgress((prev) => ({ ...prev, [folder]: "checking" }));
		}, []),
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

	const [confirmDelete, setConfirmDelete] = useState(false);
	const [pendingDelete, setPendingDelete] = useState<string[]>([]);
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
			setShowLibs((prev) => !prev);
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

		if (input === "b") {
			flashKey("b");
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
			if (showMenu) setShowMenu(false);
		}
	});

	return (
		<Box flexDirection="column" height="100%" width="100%">
			<Box flexDirection="row" gap={2}>
				<Text color="magenta" bold>
					Manage Addons
				</Text>
				{isSearching ? (
					<Box>
						<Text color="cyan">Search: </Text>
						<TextInput value={searchQuery} onChange={setSearchQuery} />
					</Box>
				) : searchQuery.length > 0 ? (
					<Box>
						<Text color="cyan">Filtered: "{searchQuery}"</Text>
						<Text color="gray"> [/] edit [esc] clear</Text>
					</Box>
				) : (
					<Text color="gray">[/] search</Text>
				)}
			</Box>

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

			<Box flexDirection="column">
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
					visibleAddons.map((item, idx) => {
						const addon = item.record;
						const isSelected = selectedIndex === idx;
						const isChecked = selectedIds.has(addon.folder);

						const query = queries[idx];
						if (!query) return null;
						const { data, isLoading, isFetching, error } = query;

						const isUpdating =
							updateMutation.isPending &&
							updateMutation.variables?.folder === addon.folder;

						let status: RepoStatus = "idle";
						let result: UpdateResult | undefined;

						if (isUpdating) {
							status = updateProgress[addon.folder] || "checking";
						} else if (isLoading || isFetching) status = "checking";
						else if (error) status = "error";
						else if (data) status = "done";

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
								key={addon.folder}
								repo={addon}
								status={status}
								result={result}
								nerdFonts={config.nerdFonts}
								isSelected={isSelected}
								isChecked={isChecked}
								isChild={item.isChild}
								isLastChild={item.isLastChild}
							/>
						);
					})
				)}
			</Box>

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
							<Text color={selectedIds.size > 0 ? "cyan" : "gray"}>
								Selected: {selectedIds.size} /{" "}
								{visibleAddons.filter((a) => !a.isChild).length}
							</Text>
							{showLibs && <Text color="gray"> [Libs: Visible]</Text>}
						</Box>
					)
				}
				controls={
					confirmDelete
						? [
								{ key: "y", label: "confirm" },
								{ key: "n", label: "cancel" },
							]
						: [
								{ key: "↑/↓", label: "nav" },
								{ key: "space", label: "select" },
								{ key: "u", label: "update" },
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
					{ key: "c", label: "Check Updates" },
					{ key: "l", label: "Toggle Libs" },
					{ key: "d", label: "Delete Selected" },
					{ key: "b", label: "Backup WTF" },
					{ key: "q", label: "Quit Application" },
					{ key: "Esc", label: "Go Back / Close" },
				]}
			/>
		</Box>
	);
};
