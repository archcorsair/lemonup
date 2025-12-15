import { Box, Text, useApp, useInput } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { useEffect, useState } from "react";
import { type Config, ConfigManager } from "../core/config";
import { AddonManager, type UpdateResult } from "../core/manager";
import {
	clearTerminalProgress,
	setTerminalProgress,
	TerminalProgressState,
} from "../core/terminal";
import { type RepoStatus, RepositoryRow } from "./components/RepositoryRow";
import { FirstRunWizard } from "./FirstRunWizard";

interface AppProps {
	force?: boolean;
	dryRun?: boolean;
}

export const App: React.FC<AppProps> = ({ force = false, dryRun = false }) => {
	const { exit } = useApp();
	const [config, setConfig] = useState<Config | null>(null);
	const [showWizard, setShowWizard] = useState(false);
	const [configManager] = useState(() => new ConfigManager()); // Memoize manager instance
	const [repoStatuses, setRepoStatuses] = useState<Record<string, RepoStatus>>(
		{},
	);
	const [results, setResults] = useState<Record<string, UpdateResult>>({});
	const [isDone, setIsDone] = useState(false);

	// Load Config
	useEffect(() => {
		if (config) return; // Already loaded

		if (!configManager.hasConfigFile) {
			setShowWizard(true);
			return;
		}

		const cfg = configManager.get();
		setConfig(cfg);

		// Initialize statuses
		const initialStatuses: Record<string, RepoStatus> = {};
		for (const repo of cfg.repositories) {
			initialStatuses[repo.name] = "idle";
		}
		setRepoStatuses(initialStatuses);
		setRepoStatuses(initialStatuses);
	}, [configManager, config]);

	// Terminal Progress
	useEffect(() => {
		if (!config) return;

		const total = config.repositories.length;
		if (total === 0) {
			clearTerminalProgress();
			return;
		}

		const completed = Object.values(repoStatuses).filter(
			(s) => s === "done" || s === "error",
		).length;

		const progress = (completed / total) * 100;
		const hasError = Object.values(repoStatuses).some((s) => s === "error");

		if (completed === total) {
			// Done
			// Leave it at 100 or clear? Usually clear after a moment, but user might want to see "Done" state.
			// Let's set it to 0 (remove) if we want to clean up, or keep it full.
			// Users usually like seeing 100%.
			// But if we exit, we should clear.
			if (hasError) {
				setTerminalProgress(TerminalProgressState.Error, 100);
			} else {
				setTerminalProgress(TerminalProgressState.Remove); // Clear on done so user knows it's finished?
				// Or maybe Running 100?
				// Ghostty/Wezterm might persist it. Let's remove it when "isDone" becomes true properly?
				// Actually, let's keep it visible until exit.
				setTerminalProgress(TerminalProgressState.Running, 100);
			}
		} else {
			setTerminalProgress(
				hasError ? TerminalProgressState.Error : TerminalProgressState.Running,
				progress,
			);
		}

		return () => {
			// Cleanup on unmount (exit)
			clearTerminalProgress();
		};
	}, [repoStatuses, config]);

	const handleWizardComplete = () => {
		setShowWizard(false);
		// Trigger reload
		const cfg = configManager.get();
		setConfig(cfg);
	};

	// Run Updates
	useEffect(() => {
		if (!config) return;

		const runUpdates = async () => {
			const manager = new AddonManager(new ConfigManager());
			// We implement a custom loop here instead of manager.updateAll to get granular state updates
			// OR we can make manager emit events. For simplicity in this iteration, let's just
			// iterate here.
			// Actually, to keep Logic in Core, we should have used updateAll but maybe refactor it to accept a callback?
			// For now, I'll allow the UI to orchestrate the loop so we can update state PER repo.

			const tempDir = await import("node:os").then((os) =>
				import("node:path").then((path) => path.join(os.tmpdir(), "lemonup")),
			);
			await import("node:fs/promises").then((fs) =>
				fs.mkdir(tempDir, { recursive: true }),
			);

			// We'll process sequentially for TUI clarity, or parallel?
			// Sequential is safer for now to avoid console fighting if logs were enabled.
			// But TUI allows parallel! Let's do parallel but that might look chaotic if we don't have a stable list.
			// We do have a stable list.

			// Let's stick to sequential for V1 stability.

			for (const repo of config.repositories) {
				setRepoStatuses((prev) => ({ ...prev, [repo.name]: "checking" }));

				// We need to access the private logic or expose public single update method.
				// manager.updateRepository is private...
				// Let's fast-fix by calling the manager logic. Ideally manager.updateAll takes a callback.

				// HACK: We will instantiate a new Manager or just rely on `updateAll` if we don't care about "Ordering".
				// BUT we want to see progress moves.
				// Let's modify Manager to be event driven later.
				// For now, I will use `updateAll` but that blocks until EVERYTHING is done. That's bad for TUI.
				// I should have exposed `updateRepository` as public? It is private in my implementation.
				// I will just use `updateAll` but wait.. that gives no feedback until end.

				// FIX: I'll assume I can just use a modified version or just accept that "Status" will be "Processing..." for all?
				// No, that's bad.

				// Let's implement the logic here for now or update Manager to public.
				// I'll update Manager in a sec. For now, assuming I can call it (TS might complain if I don't fix it).
				// I'll fix Manager to make `updateRepository` public.

				// I'll write this file assuming `manager.updateRepository` IS public (I'll patch it in next step).

				try {
					// manager updates status via callback
					const res = await manager.updateRepository(
						repo,
						config,
						tempDir,
						force,
						dryRun,
						(status) => {
							setRepoStatuses((prev) => ({
								...prev,
								[repo.name]: status as RepoStatus,
							}));
						},
					);

					setResults((prev) => ({ ...prev, [repo.name]: res }));
					setRepoStatuses((prev) => ({
						...prev,
						[repo.name]: res.success ? "done" : "error",
					}));
				} catch (_e) {
					setRepoStatuses((prev) => ({ ...prev, [repo.name]: "error" }));
				}
			}

			setIsDone(true);

			// Cleanup
			await import("node:fs/promises").then((fs) =>
				fs.rm(tempDir, { recursive: true, force: true }),
			);
		};

		runUpdates();
	}, [config, force, dryRun]);

	useInput((input, key) => {
		if (input === "q" || key.escape) {
			exit();
		}
	});

	if (showWizard) {
		return (
			<FirstRunWizard
				configManager={configManager}
				onComplete={handleWizardComplete}
			/>
		);
	}

	if (!config) {
		return <Text>Loading config...</Text>;
	}

	return (
		<Box
			flexDirection="column"
			padding={1}
			borderStyle="round"
			borderColor="cyan"
		>
			<Box
				marginBottom={1}
				borderStyle="single"
				borderColor="blue"
				paddingX={1}
			>
				<Text bold color="cyan">
					Lemonup
				</Text>
				<Box marginLeft={1}>
					<Text color="gray">v0.0.1</Text>
				</Box>
				{dryRun && (
					<Box marginLeft={2}>
						<Text color="yellow" bold>
							[DRY RUN]
						</Text>
					</Box>
				)}
			</Box>

			<Box flexDirection="column" gap={0}>
				{config.repositories.map((repo) => (
					<RepositoryRow
						key={repo.name}
						repo={repo}
						status={repoStatuses[repo.name] || "idle"}
						result={results[repo.name]}
					/>
				))}
			</Box>

			<Box marginTop={1} borderStyle="double" borderColor="gray" paddingX={1}>
				<Box flexGrow={1}>
					{isDone ? (
						<Text color="green">✔ Job's Done!</Text>
					) : (
						<Box>
							<Text color="yellow">
								<Spinner type="dots" /> Processing...
							</Text>
						</Box>
					)}
				</Box>
				<Box>
					<Text>controls: </Text>
					<Text bold color="white">
						q
					</Text>
					<Text> (quit)</Text>
				</Box>
			</Box>
		</Box>
	);
};
