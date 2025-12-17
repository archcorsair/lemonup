import { Box, Text } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import type { AddonRecord } from "../../core/db";
import type { UpdateResult } from "../../core/manager";

export type RepoStatus =
	| "idle"
	| "checking"
	| "downloading"
	| "extracting"
	| "copying"
	| "done"
	| "error";

interface RepositoryRowProps {
	repo: AddonRecord;
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
		repo.type === "tukui" ? (
			<Text color="magenta">[TukUI]</Text>
		) : repo.type === "manual" ? (
			<Text color="gray">[Man]</Text>
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
			statusText = (
				<Text color="yellow" wrap="truncate-end">
					Checking...
				</Text>
			);
			break;
		case "downloading":
			icon = nerdFonts ? (
				<Text color="cyan">
					<SpinnerFixed type="dots" />
				</Text>
			) : (
				<Text color="cyan">↓</Text>
			);
			if (repo.type === "tukui") {
				statusText = (
					<Text color="cyan" wrap="truncate-end">
						Downloading Zip...
					</Text>
				);
			} else {
				statusText = (
					<Text color="cyan" wrap="truncate-end">
						Git Syncing...
					</Text>
				);
			}
			break;
		case "extracting":
			icon = nerdFonts ? (
				<Text color="cyan">
					<SpinnerFixed type="dots" />
				</Text>
			) : (
				<Text color="cyan">E</Text>
			);
			statusText = (
				<Text color="cyan" wrap="truncate-end">
					Extracting...
				</Text>
			);
			break;
		case "copying":
			icon = nerdFonts ? (
				<Text color="cyan">
					<SpinnerFixed type="dots" />
				</Text>
			) : (
				<Text color="cyan">C</Text>
			);
			statusText = (
				<Text color="cyan" wrap="truncate-end">
					Copying...
				</Text>
			);
			break;
		case "done":
			if (result?.updated) {
				icon = <Text color="yellow">{nerdFonts ? "📦" : "Upd"}</Text>;
				statusText = (
					<Text color="yellow" wrap="truncate-end">
						{result.message}
					</Text>
				);
			} else {
				icon = <Text> </Text>;
				statusText = (
					<Text color="green" wrap="truncate-end">
						Up to date
					</Text>
				);
			}
			break;
		case "error":
			icon = <Text color="red">{nerdFonts ? "✘" : "X"}</Text>;
			statusText = (
				<Text color="red" wrap="truncate-end">
					{result?.error || "Error"}
				</Text>
			);
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

			<Box width={25}>
				<Text color="gray" wrap="truncate-end">
					{repo.version
						? repo.version.match(/^[a-f0-9]{40}$/i)
							? repo.version.substring(0, 7)
							: repo.version
						: "-"}
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
