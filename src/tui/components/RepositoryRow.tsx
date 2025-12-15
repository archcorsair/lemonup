import { Box, Text } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import type { Repository } from "../../core/config";
import type { UpdateResult } from "../../core/manager";

export type RepoStatus = "idle" | "checking" | "downloading" | "done" | "error";

interface RepositoryRowProps {
	repo: Repository;
	status: RepoStatus;
	result?: UpdateResult;
}

export const RepositoryRow: React.FC<RepositoryRowProps> = ({
	repo,
	status,
	result,
}) => {
	let icon = <Text color="gray">·</Text>;
	let statusText = <Text color="gray">Waiting</Text>;

	const typeLabel =
		repo.type === "tukui" ? (
			<Text color="magenta">[TukUI]</Text>
		) : (
			<Text color="blue">[Git]</Text>
		);

	switch (status) {
		case "idle":
			icon = <Text color="gray">·</Text>;
			statusText = <Text color="gray">Idle</Text>;
			break;
		case "checking":
			icon = (
				<Text color="yellow">
					<Spinner type="dots" />
				</Text>
			);
			statusText = <Text color="yellow">Checking...</Text>;
			break;
		case "downloading":
			// In our current App loop, we might reuse 'downloading' or stick to 'checking',
			// but assuming we expand updates to flip to this state or we treat "checking" as the active state.
			// Let's effectively treat "checking" and "downloading" as "active".
			// Since manager.ts does it all in one go, "checking" is the main state we see.
			// Ideally we want to see "Processing..." but with specificity.

			// Actually, App.tsx only sets "checking" or "done".
			// I should probably update App.tsx to set "downloading" if I want to use it?
			// Or just use "checking" here to indicate activity.
			// But for now, let's assume we want to be explicit if we CAN see it.
			// Let's just override "checking" text based on type if we are strictly checking?
			// No, Checking is usually "Checking for updates".
			// If we want to see "Downloading", manager needs to report it or we fake it.
			// But user asked for "visual difference when pulling".
			// Maybe I should change "Checking..." to "Processing..." and show the type?

			// Better: Show the Type badge always. And if "checking", say what it DOES.
			icon = (
				<Text color="cyan">
					<Spinner type="dots" />
				</Text>
			);
			if (repo.type === "tukui") {
				statusText = <Text color="cyan">Downloading Zip...</Text>;
			} else {
				statusText = <Text color="cyan">Git Syncing...</Text>;
			}
			break;
		case "done":
			if (result?.updated) {
				icon = <Text color="green">✔</Text>;
				statusText = <Text color="green">{result.message}</Text>;
			} else {
				icon = <Text color="green">✔</Text>;
				statusText = <Text color="gray">Up to date</Text>;
			}
			break;
		case "error":
			icon = <Text color="red">✘</Text>;
			statusText = <Text color="red">{result?.error || "Error"}</Text>;
			break;
	}

	return (
		<Box gap={1}>
			<Box width={3} justifyContent="center">
				{icon}
			</Box>
			<Box width={8}>{typeLabel}</Box>
			<Box width={30}>
				<Text bold>{repo.name}</Text>
			</Box>
			<Box flexGrow={1}>{statusText}</Box>
		</Box>
	);
};
