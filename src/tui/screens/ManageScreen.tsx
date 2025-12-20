import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import pLimit from "p-limit";
import type React from "react";
import { useCallback, useMemo, useState } from "react";
import { BackupManager } from "../../core/backup";
import type { Config } from "../../core/config";
import type { AddonManager, UpdateResult } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";
import { type RepoStatus, RepositoryRow } from "../components/RepositoryRow";
import { ShortcutsModal } from "../components/ShortcutsModal";
import { useKeyFeedback } from "../context/KeyFeedbackContext";
import { useAddonManagerEvent } from "../hooks/useAddonManager";

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
	const { flashKey } = useKeyFeedback();
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
	const [globalMessage, setGlobalMessage] = useState("");

	const [refreshKey, setRefreshKey] = useState(0);
	const [showLibs, setShowLibs] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: refreshKey is used to trigger re-fetching when the database state changes.
	const visibleAddons = useMemo(() => {
		const all = addonManager.getAllAddons();
		const parents = all.filter((a) => !a.parent);

		// Sort parents by name (case-insensitive)
		parents.sort((a, b) =>
			a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
		);

		if (!showLibs) {
			return parents.map((a) => ({
				record: a,
				isChild: false,
				isLastChild: false,
			}));
		}

		const result: {
			record: (typeof parents)[0];
			isChild: boolean;
			isLastChild: boolean;
		}[] = [];
		for (const p of parents) {
			result.push({ record: p, isChild: false, isLastChild: false });

			// Find children
			const children = all.filter((a) => a.parent === p.folder);
			// Sort children
			children.sort((a, b) =>
				a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
			);

			children.forEach((c, i) => {
				result.push({
					record: c,
					isChild: true,
					isLastChild: i === children.length - 1,
				});
			});
		}
		return result;
	}, [addonManager, refreshKey, showLibs]);

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

	const [updateProgress, setUpdateProgress] = useState<
		Record<string, RepoStatus>
	>({});

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

	// Mutation for Updating
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
		setGlobalMessage(
			`Updating ${foldersToUpdate.length} addon${foldersToUpdate.length > 1 ? "s" : ""}...`,
		);

		// Enforce concurrency limit
		const limit = pLimit(config.maxConcurrent);

		const promises = foldersToUpdate.map((folder) => {
			return limit(async () => {
				await updateMutation.mutateAsync({ folder });
			});
		});

		await Promise.all(promises);

		setGlobalMessage("Job's Done");
		setTimeout(() => setGlobalMessage(""), 3000);
	};

	const runChecks = async (foldersToCheck: string[]) => {
		if (!foldersToCheck.length) return;

		const limit = pLimit(config.maxConcurrent);

		// Trigger refetch for specific items

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
			setGlobalMessage("Skipped (Recently Checked)");

			setTimeout(() => setGlobalMessage(""), 2000);
		}
	};

	// Mutation for Deletion
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
		setGlobalMessage("Deleting...");
		for (const folder of foldersToDelete) {
			await deleteMutation.mutateAsync({ folder });
		}
		setRefreshKey((prev) => prev + 1); // Trigger re-render to refresh addon list
		setGlobalMessage(`Deleted ${foldersToDelete.length} addons.`);
		setTimeout(() => setGlobalMessage(""), 3000);
		setConfirmDelete(false);
	};

	const [confirmDelete, setConfirmDelete] = useState(false);
	const [pendingDelete, setPendingDelete] = useState<string[]>([]);
	const [showMenu, setShowMenu] = useState(false);

	useInput((input, key) => {
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

		const toggleMenu = () => setShowMenu((prev) => !prev);
		const closeMenu = () => setShowMenu(false);

		// If menu is open, capture keys
		if (showMenu) {
			if (key.escape || input === "m" || input === "q") {
				if (key.escape) {
					closeMenu();
					return;
				}
				closeMenu();
			}
		}

		if (key.escape) {
			if (showMenu) {
				setShowMenu(false);
				return;
			}
			onBack();
			return;
		}

		if (input === "m") {
			flashKey("m");
			toggleMenu();
			return;
		}

		if (input === "l") {
			flashKey("l");
			setShowLibs((prev) => !prev);
			setSelectedIndex(0); // Reset selection to avoid bounds error
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
				setGlobalMessage("Backing up WTF...");
				try {
					const result = await BackupManager.backupWTF(config.destDir);

					if (result === null) {
						setGlobalMessage("Backup Failed: WTF folder not found");
					} else if (result === "skipped-recent") {
						setGlobalMessage("Backup Skipped (Too Recent)");
					} else {
						// Result is the path to the zip file
						await BackupManager.cleanupBackups(
							config.destDir,
							config.backupRetention,
						);
						setGlobalMessage("Backup Complete!");
					}
					setTimeout(() => setGlobalMessage(""), 3000);
				} catch (error) {
					setGlobalMessage(`Backup Failed: ${(error as Error).message}`);
				}
			};
			runBackup();
			if (showMenu) setShowMenu(false);
		}
	});

	return (
		<Box flexDirection="column" padding={1} height="100%" width="100%">
			<Text color="magenta" bold>
				Manage Addons
			</Text>

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
					<Text bold>Status</Text>
				</Box>
				<Box flexGrow={2} flexShrink={1} minWidth={15} flexBasis="20%">
					<Text bold>Name</Text>
				</Box>
				<Box flexGrow={1} flexShrink={1} minWidth={10} flexBasis="15%">
					<Text bold>Author</Text>
				</Box>
				<Box width={8} flexShrink={0}>
					<Text bold>Source</Text>
				</Box>
			</Box>

			<Box flexDirection="column">
				{visibleAddons.length === 0 ? (
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
					) : globalMessage ? (
						globalMessage.includes("Done") ||
						globalMessage.includes("Complete") ||
						globalMessage.includes("Deleted") ? (
							<Text color="green">✔ {globalMessage}</Text>
						) : globalMessage.includes("Skipped") ? (
							<Text color="cyan">ℹ {globalMessage}</Text>
						) : (
							<Text color="yellow">
								{/* @ts-expect-error: Spinner types mismatch */}
								<Spinner type="dots" /> {globalMessage}
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
								{ key: "l", label: showLibs ? "hide libs" : "show libs" },
								{ key: "m", label: "menu" },
							]
				}
			/>

			<ShortcutsModal
				visible={showMenu}
				shortcuts={[
					{ key: "↑/↓", label: "Navigate" },
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
