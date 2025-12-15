import { Box, Text } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { REPO_TYPE, type Repository } from "../../core/config";

import type { UpdateResult } from "../../core/manager";

export type RepoStatus = "idle" | "checking" | "downloading" | "done" | "error";

interface RepositoryRowProps {
	repo: Repository;
	status: RepoStatus;
	result?: UpdateResult;
	nerdFonts?: boolean;
	isSelected?: boolean;
	isChecked?: boolean;
}

export const RepositoryRow: React.FC<RepositoryRowProps> = ({
	repo,
	status,
	result,
	nerdFonts = true,
	isSelected = false,
	isChecked = false,
}) => {
	let icon = <Text color="gray">·</Text>;
	let statusText = <Text color="gray">Waiting</Text>;

	const typeLabel =
		repo.type === REPO_TYPE.TUKUI ? (
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
			icon = nerdFonts ? (
				<Text color="yellow">
					<SpinnerFixed type="dots" />
				</Text>
			) : (
				<Text color="yellow">?</Text>
			);
			statusText = <Text color="yellow">Checking...</Text>;
			break;
		case "downloading":
			icon = nerdFonts ? (
				<Text color="cyan">
					<SpinnerFixed type="dots" />
				</Text>
			) : (
				<Text color="cyan">↓</Text>
			);
			if (repo.type === REPO_TYPE.TUKUI) {
				statusText = <Text color="cyan">Downloading Zip...</Text>;
			} else {
				statusText = <Text color="cyan">Git Syncing...</Text>;
			}
			break;
		case "done":
			if (result?.updated) {
				icon = <Text color="green">{nerdFonts ? "✔" : "OK"}</Text>;
				statusText = <Text color="green">{result.message}</Text>;
			} else {
				icon = <Text> </Text>;
				statusText = <Text color="white">Up to date</Text>;
			}
			break;
		case "error":
			icon = <Text color="red">{nerdFonts ? "✘" : "X"}</Text>;
			statusText = <Text color="red">{result?.error || "Error"}</Text>;
			break;
	}

	return (
		<Box paddingX={1}>
			<Box width={4}>
				<Text color="blue">{isSelected ? "> " : "  "}</Text>
				<Text color={isChecked ? "green" : "gray"}>
					{isChecked ? (nerdFonts ? "●" : "*") : " "}
				</Text>
			</Box>

			<Box width={20}>
				<Text color={isSelected ? "blue" : isChecked ? "green" : undefined}>
					{repo.name}
				</Text>
			</Box>

			<Box width={10}>{typeLabel}</Box>

			<Box width={15}>
				<Text color="gray">
					{repo.installedVersion ? repo.installedVersion.substring(0, 7) : "-"}
				</Text>
			</Box>

			<Box width={30}>
				<Box gap={1}>
					<Box width={3} justifyContent="center">
						{icon}
					</Box>
					<Box flexGrow={1}>{statusText}</Box>
				</Box>
			</Box>
		</Box>
	);
};

// Workaround for React 19 + Ink type mismatch
const SpinnerFixed = Spinner as unknown as React.FC<{
	type?: string;
}>;
