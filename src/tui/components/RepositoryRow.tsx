import { Box, Text } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import type { AddonRecord } from "@/core/db";
import type { UpdateResult } from "@/core/manager";

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
	isChild?: boolean;
	isLastChild?: boolean;
}

export const RepositoryRow: React.FC<RepositoryRowProps> = ({
	repo,
	status,
	result,
	nerdFonts = true,
	isSelected = false,
	isChecked = false,
	isChild = false,
	isLastChild = false,
}) => {
	let icon = <Text color="gray">·</Text>;
	let statusText = <Text color="gray">Waiting</Text>;

	const typeLabel =
		repo.type === "tukui" ? (
			<Text color="magenta">[TukUI]</Text>
		) : repo.type === "wowinterface" ? (
			<Text color="yellow">[WoWI]</Text>
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
			if (repo.type === "tukui" || repo.type === "wowinterface") {
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
				// Distinguish between "Update Available" (ManageScreen) and "Updated" (UpdateScreen)
				// ManageScreen uses "Update: ..." message convention
				const isUpdateAvailableMsg = result.message?.startsWith("Update:");

				if (isUpdateAvailableMsg) {
					icon = <Text color="yellow">{nerdFonts ? "📦" : "!"}</Text>;
					statusText = (
						<Text color="yellow" wrap="truncate-end">
							{result.message}
						</Text>
					);
				} else {
					// "Updated to ..." - Success
					icon = <Text color="green">{nerdFonts ? "✔" : "OK"}</Text>;
					statusText = (
						<Text color="green" wrap="truncate-end">
							{result.message}
						</Text>
					);
				}
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

	const displayVersion = repo.version
		? repo.version.match(/^[a-f0-9]{40}$/i)
			? repo.version.substring(0, 7)
			: repo.version
		: "";

	// Tree View Indentation
	let namePrefix = null;
	if (isChild) {
		namePrefix = <Text color="gray">{isLastChild ? "└── " : "├── "}</Text>;
	}

	return (
		<Box paddingX={2} width="100%">
			<Box width={3} flexShrink={0}>
				<Text color="blue">{isSelected ? ">" : " "}</Text>
				<Text color={isChecked ? "green" : "gray"}>
					{isChecked ? (nerdFonts ? "●" : "*") : " "}
				</Text>
			</Box>

			<Box width={22} flexShrink={0}>
				{!isChild ? (
					<Box gap={1} flexDirection="row">
						<Box flexGrow={1} flexShrink={1}>
							{statusText}
						</Box>
						<Box width={2} justifyContent="flex-end">
							{icon}
						</Box>
					</Box>
				) : (
					<Text> </Text>
				)}
			</Box>

			<Box flexGrow={2} flexShrink={1} minWidth={15} flexBasis="20%">
				{namePrefix}
				<Text
					color={isSelected ? "blue" : isChecked ? "green" : undefined}
					wrap="truncate-end"
				>
					{repo.name}{" "}
					{repo.kind === "library" && (
						<Text color="cyan" dimColor>
							[Lib{repo.kindOverride ? "*" : ""}]
						</Text>
					)}
					{displayVersion ? <Text color="gray">({displayVersion})</Text> : null}
				</Text>
			</Box>

			<Box flexGrow={1} flexShrink={1} minWidth={10} flexBasis="15%">
				<Text wrap="truncate-end">{repo.author || "-"}</Text>
			</Box>

			<Box width={8} flexShrink={0}>
				{typeLabel}
			</Box>
		</Box>
	);
};

// Workaround for React 19 + Ink type mismatch
const SpinnerFixed = Spinner as unknown as React.FC<{
	type?: string;
}>;
