import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { useEffect, useState } from "react";
import type { Config } from "../../core/config";
import type { AddonManager, UpdateResult } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";
import { type RepoStatus, RepositoryRow } from "../components/RepositoryRow";

interface UpdateScreenProps {
	config: Config;
	addonManager: AddonManager;
	force?: boolean;
	dryRun?: boolean;
	testMode?: boolean;
	onBack: () => void;
}

export const UpdateScreen: React.FC<UpdateScreenProps> = ({
	config: initialConfig,
	addonManager,
	force = false,
	dryRun = false,
	testMode = false,
	onBack,
}) => {
	const { exit } = useApp();
	const [config] = useState(initialConfig);

	const [repoStatuses, setRepoStatuses] = useState<Record<string, RepoStatus>>(
		{},
	);
	const [results, setResults] = useState<Record<string, UpdateResult>>({});
	const [isDone, setIsDone] = useState(false);

	useEffect(() => {
		const initialStatuses: Record<string, RepoStatus> = {};
		for (const repo of config.repositories) {
			initialStatuses[repo.name] = "idle";
		}
		setRepoStatuses(initialStatuses);
	}, [config]);

	const [backupStatus, setBackupStatus] = useState<
		"idle" | "running" | "success" | "error" | "skipped"
	>("idle");

	useEffect(() => {
		const runUpdates = async () => {
			const tempDir = await import("node:os").then((os) =>
				import("node:path").then((path) => path.join(os.tmpdir(), "lemonup")),
			);
			await import("node:fs/promises").then((fs) =>
				fs.mkdir(tempDir, { recursive: true }),
			);

			const freshConfig = addonManager.getConfig();
			const allAddons = addonManager.getAllAddons();

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

			for (const addon of allAddons) {
				setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "checking" }));

				try {
					const res = await addonManager.updateAddon(
						addon,
						freshConfig,
						tempDir,
						force,
						dryRun,
						(status: string) => {
							setRepoStatuses((prev) => ({
								...prev,
								[addon.folder]: status as RepoStatus,
							}));
						},
					);

					setResults((prev) => ({ ...prev, [addon.folder]: res }));
					setRepoStatuses((prev) => ({
						...prev,
						[addon.folder]: res.success ? "done" : "error",
					}));
				} catch (e: unknown) {
					const errorMsg = e instanceof Error ? e.message : String(e);
					setResults((prev) => ({
						...prev,
						[addon.folder]: {
							repoName: addon.name,
							success: false,
							updated: false,
							error: errorMsg,
						},
					}));
					setRepoStatuses((prev) => ({ ...prev, [addon.folder]: "error" }));
				}
			}

			setIsDone(true);

			await import("node:fs/promises").then((fs) =>
				fs.rm(tempDir, { recursive: true, force: true }),
			);
		};

		runUpdates();
	}, [addonManager, force, dryRun, testMode]);

	useInput((input, key) => {
		if (key.escape || (input === "q" && isDone)) {
			if (key.escape) onBack();
			else if (input === "q") exit();
		}
	});

	// We need to render based on database state, not config
	// But State here is local. We should init state from allAddons?
	// The effect runs on mount.
	// Let's grab addons outside for rendering
	const allAddons = addonManager.getAllAddons();

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
