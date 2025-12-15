import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";
import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import pLimit from "p-limit";
import type React from "react";
import { useState } from "react";
import { BackupManager } from "../../core/backup";
import type { Config, Repository } from "../../core/config";
import type { AddonManager, UpdateResult } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";
import { type RepoStatus, RepositoryRow } from "../components/RepositoryRow";
import { ShortcutsModal } from "../components/ShortcutsModal";

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

	// 1. Queries for Status Checking
	const queries = useQueries({
		queries: config.repositories.map((repo) => ({
			queryKey: ["addon", repo.name],
			queryFn: async () => {
				// Fetch fresh config to ensure we check against the potentially updated installedVersion
				const freshConfig = addonManager.getConfig();
				const freshRepo = freshConfig.repositories.find(
					(r) => r.name === repo.name,
				);
				if (!freshRepo)
					return {
						updateAvailable: false,
						remoteVersion: "",
						error: "Repo not found",
						checkedVersion: null,
					};

				const res = await addonManager.checkUpdate(freshRepo);
				return { ...res, checkedVersion: freshRepo.installedVersion };
			},
			staleTime: config.checkInterval, // User configured check interval
		})),
	});

	// Track granular progress for updates
	const [updateProgress, setUpdateProgress] = useState<
		Record<string, RepoStatus>
	>({});

	// 2. Mutation for Updating
	const updateMutation = useMutation({
		mutationFn: async ({ repo }: { repo: Repository }) => {
			const os = await import("node:os");
			const path = await import("node:path");
			const fs = await import("node:fs/promises");

			const tempDir = path.join(os.tmpdir(), "lemonup-manage-single");
			await fs.mkdir(tempDir, { recursive: true });

			try {
				const result = await addonManager.updateRepository(
					repo,
					addonManager.getConfig(),
					tempDir,
					force,
					dryRun,
					(status) => {
						setUpdateProgress((prev) => ({
							...prev,
							[repo.name]: status as RepoStatus,
						}));
					},
				);
				return result;
			} finally {
				await fs.rm(tempDir, { recursive: true, force: true }).catch(() => {});
				// Clean up progress
				setUpdateProgress((prev) => {
					const next = { ...prev };
					delete next[repo.name];
					return next;
				});
			}
		},
		onSuccess: (_, variables) => {
			queryClient.invalidateQueries({
				queryKey: ["addon", variables.repo.name],
			});
		},
	});

	// Helpers
	const runUpdates = async (idsToUpdate: string[]) => {
		if (idsToUpdate.length === 0) return;
		setGlobalMessage(
			`Updating ${idsToUpdate.length} addon${idsToUpdate.length > 1 ? "s" : ""}...`,
		);

		// Run with concurrency limit
		const limit = pLimit(config.maxConcurrent);

		const promises = idsToUpdate.map((name) => {
			return limit(async () => {
				const repo = config.repositories.find((r) => r.name === name);
				if (repo) {
					await updateMutation.mutateAsync({ repo });
				}
			});
		});

		await Promise.all(promises);

		setGlobalMessage("Job's Done");
		setTimeout(() => setGlobalMessage(""), 3000);
	};

	const runChecks = async (idsToCheck: string[]) => {
		if (!idsToCheck.length) return;

		// Run with concurrency limit
		const limit = pLimit(config.maxConcurrent);

		// Trigger refetch for specific items
		const promises = idsToCheck.map((name) => {
			return limit(async () => {
				const idx = config.repositories.findIndex((r) => r.name === name);
				if (idx !== -1 && queries[idx]) {
					await queries[idx].refetch();
				}
			});
		});

		await Promise.all(promises);
	};

	// Menu state
	const [showMenu, setShowMenu] = useState(false);

	useInput((input, key) => {
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

		if (showMenu && ["k", "j", " ", "u", "c"].includes(input)) {
			setShowMenu(false);
		}
		if (showMenu && (key.upArrow || key.downArrow)) {
			setShowMenu(false);
		}

		if (key.upArrow || input === "k") {
			setSelectedIndex((prev) => Math.max(0, prev - 1));
		}

		if (key.downArrow || input === "j") {
			setSelectedIndex((prev) =>
				Math.min(config.repositories.length - 1, prev + 1),
			);
		}

		if (input === " ") {
			const currentRepo = config.repositories[selectedIndex];
			if (currentRepo) {
				setSelectedIds((prev) => {
					const next = new Set(prev);
					if (next.has(currentRepo.name)) {
						next.delete(currentRepo.name);
					} else {
						next.add(currentRepo.name);
					}
					return next;
				});
			}
		}

		if (input === "u") {
			if (selectedIds.size > 0) {
				runUpdates(Array.from(selectedIds));
			} else {
				const currentRepo = config.repositories[selectedIndex];
				if (currentRepo) {
					runUpdates([currentRepo.name]);
				}
			}
		}

		if (input === "c") {
			if (selectedIds.size > 0) {
				runChecks(Array.from(selectedIds));
			} else {
				const currentRepo = config.repositories[selectedIndex];
				if (currentRepo) {
					runChecks([currentRepo.name]);
				}
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
			>
				<Box width={4}>
					<Text> </Text>
				</Box>
				<Box width={20}>
					<Text bold>Name</Text>
				</Box>
				<Box width={10}>
					<Text bold>Source</Text>
				</Box>
				<Box width={15}>
					<Text bold>Installed</Text>
				</Box>
				<Box width={30}>
					<Text bold>Status</Text>
				</Box>
			</Box>

			<Box flexDirection="column">
				{config.repositories.map((repo, idx) => {
					const isSelected = selectedIndex === idx;
					const isChecked = selectedIds.has(repo.name);

					const query = queries[idx];
					if (!query) return null;
					const { data, isLoading, isFetching, error } = query;

					const isUpdating =
						updateMutation.isPending &&
						updateMutation.variables?.repo.name === repo.name;

					let status: RepoStatus = "idle";
					let result: UpdateResult | undefined;

					if (isUpdating) {
						status = updateProgress[repo.name] || "checking";
					} else if (isLoading || isFetching) status = "checking";
					else if (error) status = "error";
					else if (data) status = "done";

					// Mock result for "done" state based on query data
					if (status === "done" && data) {
						if (data.updateAvailable) {
							result = {
								repoName: repo.name,
								success: true,
								updated: true,
								// DEBUG: Show versions
								message: `Update Avail: ${String(data.checkedVersion).substring(0, 7)} -> ${String(data.remoteVersion).substring(0, 7)}`,
							};
						} else {
							result = {
								repoName: repo.name,
								success: true,
								updated: false,
								message: "Up to date",
							};
						}
					}

					return (
						<RepositoryRow
							key={repo.name}
							repo={repo}
							status={status}
							result={result}
							nerdFonts={config.nerdFonts}
							isSelected={isSelected}
							isChecked={isChecked}
						/>
					);
				})}
			</Box>

			<ControlBar
				message={
					globalMessage ? (
						globalMessage.includes("Done") ||
						globalMessage.includes("Complete") ? (
							<Text color="green">✔ {globalMessage}</Text>
						) : (
							<Text color="yellow">
								{/* @ts-expect-error: Spinner types mismatch */}
								<Spinner type="dots" /> {globalMessage}
							</Text>
						)
					) : (
						<Text color="gray">
							Selected: {selectedIds.size} / {config.repositories.length}
						</Text>
					)
				}
				controls={[
					{ key: "↑/↓", label: "nav" },
					{ key: "space", label: "select" },
					{ key: "u", label: "update" },
					{ key: "c", label: "check" },
					{ key: "m", label: "menu" },
				]}
			/>

			<ShortcutsModal
				visible={showMenu}
				shortcuts={[
					{ key: "↑/↓", label: "Navigate" },
					{ key: "Space", label: "Select/Deselect" },
					{ key: "u", label: "Update Selected" },
					{ key: "c", label: "Check Updates" },
					{ key: "b", label: "Backup WTF" },
					{ key: "q", label: "Quit Application" },
					{ key: "Esc", label: "Go Back / Close" },
				]}
			/>
		</Box>
	);
};
