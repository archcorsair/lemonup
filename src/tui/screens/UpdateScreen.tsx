import { useQueryClient } from "@tanstack/react-query";
import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { UpdateAddonResult } from "../../core/commands/UpdateAddonCommand";
import type { Config } from "../../core/config";
import type { AddonManager } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";
import { type RepoStatus, RepositoryRow } from "../components/RepositoryRow";
import { useAddonManagerEvent } from "../hooks/useAddonManager";
import { useAppStore } from "../store/useAppStore";

interface UpdateScreenProps {
	config: Config;
	addonManager: AddonManager;
	force?: boolean;
	dryRun?: boolean;
	testMode?: boolean;
	onBack: () => void;
}

export const UpdateScreen: React.FC<UpdateScreenProps> = ({
	config,
	addonManager,
	force = false,
	testMode = false,
	onBack,
}) => {
	const { exit } = useApp();
	const flashKey = useAppStore((state) => state.flashKey);
	const queryClient = useQueryClient();

	const [repoStatuses, setRepoStatuses] = useState<Record<string, RepoStatus>>(
		{},
	);
	const [results, setResults] = useState<Record<string, UpdateAddonResult>>({});
	const [isDone, setIsDone] = useState(false);
	const [backupStatus, setBackupStatus] = useState<
		"idle" | "running" | "success" | "error" | "skipped"
	>("idle");

	const allAddons = useMemo(() => addonManager.getAllAddons(), [addonManager]);
	const hasRun = useRef(false);
	const remoteVersions = useRef<Record<string, string>>({});

	useAddonManagerEvent(
		addonManager,
		"addon:update-check:complete",
		(folder, _updateAvailable, remoteVersion) => {
			remoteVersions.current[folder] = remoteVersion;
		},
	);

	useAddonManagerEvent(addonManager, "addon:update-check:start", (folder) => {
		const addon = allAddons.find((a) => a.folder === folder);
		if (addon) {
			setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "checking" }));
		}
	});

	useAddonManagerEvent(addonManager, "addon:install:downloading", (folder) => {
		const addon = allAddons.find((a) => a.folder === folder);
		if (addon) {
			setRepoStatuses((prev) => ({
				...prev,
				[addon.folder]: "downloading",
			}));
		}
	});

	useAddonManagerEvent(addonManager, "addon:install:extracting", (folder) => {
		const addon = allAddons.find((a) => a.folder === folder);
		if (addon) {
			setRepoStatuses((prev) => ({
				...prev,
				[addon.folder]: "extracting",
			}));
		}
	});

	useAddonManagerEvent(addonManager, "addon:install:copying", (folder) => {
		const addon = allAddons.find((a) => a.folder === folder);
		if (addon) {
			setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "copying" }));
		}
	});

	useEffect(() => {
		if (hasRun.current) return;
		hasRun.current = true;

		const runUpdates = async () => {
			const freshConfig = addonManager.getConfig();

			if (freshConfig.backupWTF) {
				setBackupStatus("running");
				try {
					const { BackupManager } = await import("../../core/backup");
					if (testMode) {
						const retailDir = await import("node:path").then((path) =>
							path.dirname(path.dirname(freshConfig.destDir)),
						);
						const wtfDir = await import("node:path").then((path) =>
							path.join(retailDir, "WTF"),
						);
						await import("node:fs/promises").then((fs) =>
							fs.mkdir(wtfDir, { recursive: true }),
						);
					}

					const result = await BackupManager.backupWTF(freshConfig.destDir, 15);

					if (result === "skipped-recent") {
						setBackupStatus("skipped");
					} else {
						await BackupManager.cleanupBackups(
							freshConfig.destDir,
							freshConfig.backupRetention,
						);
						setBackupStatus("success");
					}
				} catch {
					setBackupStatus("error");
				}
			} else {
				setBackupStatus("skipped");
			}

			// Manual update loop to support caching
			const updateResults: UpdateAddonResult[] = [];
			const resultsMap: Record<string, UpdateAddonResult> = {};
			const statusMap: Record<string, RepoStatus> = {};

			for (const addon of allAddons) {
				if (addon.type === "manual") continue;

				const queryKey = ["addon", addon.folder];
				const state = queryClient.getQueryState(queryKey);
				const now = Date.now();
				const dataUpdatedAt = state?.dataUpdatedAt ?? 0;
				const cachedData = state?.data as
					| { updateAvailable: boolean; remoteVersion: string }
					| undefined;

				if (
					!force &&
					cachedData &&
					dataUpdatedAt > 0 &&
					now - dataUpdatedAt < freshConfig.checkInterval &&
					!cachedData.updateAvailable
				) {
					const res: UpdateAddonResult = {
						repoName: addon.name,
						success: true,
						updated: false,
						message: "Skipped (Recently Checked)",
					};
					updateResults.push(res);
					resultsMap[addon.folder] = res;
					statusMap[addon.folder] = "done";
					setResults((prev) => ({ ...prev, [addon.folder]: res }));
					setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "done" }));
					continue;
				}

				try {
					const result = await addonManager.updateAddon(addon, force);
					updateResults.push(result);
					resultsMap[addon.folder] = result;
					statusMap[addon.folder] = result.success ? "done" : "error";

					setResults((prev) => ({ ...prev, [addon.folder]: result }));
					setRepoStatuses((prev) => ({
						...prev,
						[addon.folder]: result.success ? "done" : "error",
					}));

					const remoteVer = remoteVersions.current[addon.folder] || "";
					queryClient.setQueryData(queryKey, {
						updateAvailable: false,
						remoteVersion: remoteVer,
						checkedVersion: addon.version,
					});
				} catch (error) {
					const res = {
						repoName: addon.name,
						success: false,
						updated: false,
						error: String(error),
					};
					updateResults.push(res);
					resultsMap[addon.folder] = res;
					statusMap[addon.folder] = "error";
					setResults((prev) => ({ ...prev, [addon.folder]: res }));
					setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "error" }));
				}
			}

			setIsDone(true);
		};

		runUpdates();
	}, [addonManager, force, testMode, allAddons, queryClient]);

	useInput((input, key) => {
		if (key.escape || (input === "q" && isDone)) {
			if (key.escape) {
				flashKey("esc");
				onBack();
			} else if (input === "q") {
				flashKey("q");
				exit();
			}
		}
	});

	return (
		<Box flexDirection="column" gap={0}>
			{backupStatus !== "skipped" && backupStatus !== "idle" && (
				<Box
					borderStyle="single"
					borderColor={
						backupStatus === "success"
							? "green"
							: backupStatus === "error"
								? "red"
								: "yellow"
					}
					paddingX={1}
					marginBottom={1}
				>
					<Text>
						Settings Backup:{" "}
						{backupStatus === "running" && (
							<Text color="yellow">⏳ In Progress...</Text>
						)}
						{backupStatus === "success" && <Text color="green">✔ Done</Text>}
						{backupStatus === "error" && <Text color="red">✘ Failed</Text>}
					</Text>
				</Box>
			)}

			{allAddons.map((addon) => (
				<RepositoryRow
					key={addon.folder}
					repo={addon}
					status={repoStatuses[addon.folder] || "idle"}
					result={results[addon.folder]}
					nerdFonts={config.nerdFonts}
				/>
			))}

			<ControlBar
				message={
					isDone ? (
						<Text color="green">✔ Job's Done!</Text>
					) : (
						<Box>
							<Text color="yellow">
								{config.nerdFonts ? (
									<>
										{/* @ts-expect-error: Spinner types mismatch */}
										<Spinner type="dots" />{" "}
									</>
								) : null}
								{backupStatus === "running" ? "Backing up..." : "Processing..."}
							</Text>
						</Box>
					)
				}
				controls={[
					{ key: "q", label: "quit" },
					{ key: "esc", label: "back" },
				]}
			/>
		</Box>
	);
};
