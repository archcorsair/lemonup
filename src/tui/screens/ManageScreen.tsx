import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import pLimit from "p-limit";
import type React from "react";
import { useState } from "react";
import { BackupManager } from "../../core/backup";
import type { Config } from "../../core/config";
import type { AddonManager, UpdateResult } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";
import { type RepoStatus, RepositoryRow } from "../components/RepositoryRow";
import { ShortcutsModal } from "../components/ShortcutsModal";
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
	dryRun = false,
	onBack,
}) => {
	const { exit } = useApp();
	const queryClient = useQueryClient();
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
	const [globalMessage, setGlobalMessage] = useState("");

	// Force re-render after DB changes
	const [refreshKey, setRefreshKey] = useState(0);

	// 1. Fetch Addons from DB
	// Dependent on refreshKey to ensure re-fetch on update
	const addons = addonManager.getAllAddons();

	// 1. Queries for Status Checking
	const queries = useQueries({
		queries: addons.map((addon) => ({
			queryKey: ["addon", addon.folder],
			queryFn: async () => {
				const freshAddon = addonManager.getAddon(addon.folder);
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
			staleTime: config.checkInterval, // User configured check interval
		})),
	});

	// Track granular progress for updates
	const [updateProgress, setUpdateProgress] = useState<
		Record<string, RepoStatus>
	>({});

	useAddonManagerEvent(
		addonManager,
		"addon:update-check:start",
		(name) => {
			setUpdateProgress((prev) => ({ ...prev, [name]: "checking" }));
		},
	);
	useAddonManagerEvent(
		addonManager,
		"addon:install:downloading",
		(name) => {
			setUpdateProgress((prev) => ({ ...prev, [name]: "downloading" }));
		},
	);
	useAddonManagerEvent(
		addonManager,
		"addon:install:extracting",
		(name) => {
			setUpdateProgress((prev) => ({ ...prev, [name]: "extracting" }));
		},
	);
	useAddonManagerEvent(
		addonManager,
		"addon:install:copying",
		(name) => {
			setUpdateProgress((prev) => ({ ...prev, [name]: "copying" }));
		},
	);
	useAddonManagerEvent(
		addonManager,
		"addon:install:complete",
		(name) => {
			setUpdateProgress((prev) => {
				const next = { ...prev };
				delete next[name];
				return next;
			});
		},
	);

	// 2. Mutation for Updating
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
				const result = await addonManager.updateAddon(
					addon,
					force,
				);
				return result;
			} finally {
				await fs.rm(tempDir, { recursive: true, force: true }).catch(() => {});
				// Clean up progress
				setUpdateProgress((prev) => {
					const next = { ...prev };
					delete next[addon.name];
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

		// Run with concurrency limit
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

		// Run with concurrency limit
		const limit = pLimit(config.maxConcurrent);

		// Trigger refetch for specific items
		const promises = foldersToCheck.map((folder) => {
			return limit(async () => {
				const idx = addons.findIndex((r) => r.folder === folder);
				if (idx !== -1 && queries[idx]) {
					await queries[idx].refetch();
				}
			});
		});

		await Promise.all(promises);
	};

	// 3. Mutation for Deletion
	const deleteMutation = useMutation({
		mutationFn: async ({ folder }: { folder: string }) => {
			await addonManager.removeAddon(folder);
		},
		onSuccess: (_, variables) => {
			// Uncheck removed item
			if (selectedIds.has(variables.folder)) {
				setSelectedIds((prev) => {
					const next = new Set(prev);
					next.delete(variables.folder);
					return next;
				});
			}
			// Invalidate to refresh list
			// We need to refetch the whole list or force re-render?
			// getAllAddons is direct call. We must trigger re-render of component.
			// Since `addons` is const computed on render, assume invalidating queries might not be enough if addons list itself isn't a query.
			// `addons` comes from `addonManager.getAllAddons()` which is SYNC.
			// But we need to force re-render.
			// We can do this by using a dummy state or making addons a query.
			// For now, let's assume parent re-renders or we force it.
			// Actually, `addons` is not state, it's derived. We need to force update.
			// Let's verify if Tanstack Query invalidation helps here? No, addons is not from query.
			// We should ideally wrap `getAllAddons` in a query.
			// Or simpler: increment a version counter.
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

	// CONFIRMATION STATE
	const [confirmDelete, setConfirmDelete] = useState(false);
	const [pendingDelete, setPendingDelete] = useState<string[]>([]);

	// Menu state
	const [showMenu, setShowMenu] = useState(false);

	useInput((input, key) => {
		if (confirmDelete) {
			if (input === "y" || key.return) {
				runDeletes(pendingDelete);
			} else if (input === "n" || key.escape) {
				setConfirmDelete(false);
				setPendingDelete([]);
			}
			return;
		}

		// Shortcuts for menu
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
			toggleMenu();
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

		if (key.upArrow || input === "k") {
			setSelectedIndex((prev) => Math.max(0, prev - 1));
		}

		if (key.downArrow || input === "j") {
			setSelectedIndex((prev) => Math.min(addons.length - 1, prev + 1));
		}

		if (input === " ") {
			const currentAddon = addons[selectedIndex];
			if (currentAddon) {
				setSelectedIds((prev) => {
					const next = new Set(prev);
					if (next.has(currentAddon.folder)) {
						next.delete(currentAddon.folder);
					} else {
						next.add(currentAddon.folder);
					}
					return next;
				});
			}
		}

		if (input === "u") {
			if (selectedIds.size > 0) {
				runUpdates(Array.from(selectedIds));
			} else {
				const currentAddon = addons[selectedIndex];
				if (currentAddon) {
					runUpdates([currentAddon.folder]);
				}
			}
		}

		if (input === "c") {
			if (selectedIds.size > 0) {
				runChecks(Array.from(selectedIds));
			} else {
				const currentAddon = addons[selectedIndex];
				if (currentAddon) {
					runChecks([currentAddon.folder]);
				}
			}
		}

		if (input === "d" || key.delete) {
			const targets =
				selectedIds.size > 0
					? Array.from(selectedIds)
					: addons[selectedIndex]
						? [addons[selectedIndex].folder]
						: [];
			if (targets.length > 0) {
				setPendingDelete(targets);
				setConfirmDelete(true);
			}
		}

		if (input === "b") {
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
				<Box width={4} flexShrink={0}>
					<Text> </Text>
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
				<Box width={12} flexShrink={0}>
					<Text bold>Version</Text>
				</Box>
				<Box flexGrow={1} flexShrink={1} minWidth={15} flexBasis="25%">
					<Text bold>Status</Text>
				</Box>
			</Box>

			<Box flexDirection="column">
				{addons.length === 0 ? (
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
					addons.map((addon, idx) => {
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
							status = updateProgress[addon.name] || "checking";
						} else if (isLoading || isFetching) status = "checking";
						else if (error) status = "error";
						else if (data) status = "done";

						// Mock result for "done" state based on query data
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
						) : (
							<Text color="yellow">
								{/* @ts-expect-error: Spinner types mismatch */}
								<Spinner type="dots" /> {globalMessage}
							</Text>
						)
					) : (
						<Text color="gray">
							Selected: {selectedIds.size} / {addons.length}
						</Text>
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
								{ key: "d", label: "delete" },
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
					{ key: "d", label: "Delete Selected" },
					{ key: "b", label: "Backup WTF" },
					{ key: "q", label: "Quit Application" },
					{ key: "Esc", label: "Go Back / Close" },
				]}
			/>
		</Box>
	);
};
