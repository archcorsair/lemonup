import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { UpdateAddonResult } from "../../core/commands/UpdateAddonCommand";
import type { Config } from "../../core/config";
import type { AddonManager } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";
import { type RepoStatus, RepositoryRow } from "../components/RepositoryRow";
import { useAddonManagerEvent } from "../hooks/useAddonManager";

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

	// Event Handlers
	useAddonManagerEvent(
		addonManager,
		"addon:update-check:start",
		useCallback(
			(name) => {
				const addon = allAddons.find((a) => a.name === name);
				if (addon) {
					setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "checking" }));
				}
			},
			[allAddons],
		),
	);

	useAddonManagerEvent(
		addonManager,
		"addon:install:downloading",
		useCallback(
			(name) => {
				const addon = allAddons.find((a) => a.name === name);
				if (addon) {
					setRepoStatuses((prev) => ({
						...prev,
						[addon.folder]: "downloading",
					}));
				}
			},
			[allAddons],
		),
	);

	useAddonManagerEvent(
		addonManager,
		"addon:install:extracting",
		useCallback(
			(name) => {
				const addon = allAddons.find((a) => a.name === name);
				if (addon) {
					setRepoStatuses((prev) => ({
						...prev,
						[addon.folder]: "extracting",
					}));
				}
			},
			[allAddons],
		),
	);

	useAddonManagerEvent(
		addonManager,
		"addon:install:copying",
		useCallback(
			(name) => {
				const addon = allAddons.find((a) => a.name === name);
				if (addon) {
					setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "copying" }));
				}
			},
			[allAddons],
		),
	);

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

			const updateResults = await addonManager.updateAll(force);

			// Map results to folders for rendering
			const resultsMap: Record<string, UpdateAddonResult> = {};
			const statusMap: Record<string, RepoStatus> = {};

			for (const res of updateResults) {
				const addon = allAddons.find((a) => a.name === res.repoName);
				if (addon) {
					resultsMap[addon.folder] = res;
					statusMap[addon.folder] = res.success ? "done" : "error";
				}
			}

			setResults(resultsMap);
			setRepoStatuses((prev) => ({ ...prev, ...statusMap }));
			setIsDone(true);
		};

		runUpdates();
	}, [addonManager, force, testMode, allAddons]);

	useInput((input, key) => {
		if (key.escape || (input === "q" && isDone)) {
			if (key.escape) onBack();
			else if (input === "q") exit();
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
