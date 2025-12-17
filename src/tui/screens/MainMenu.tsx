import { Box, Text, useApp, useInput } from "ink";
import type React from "react";
import { useState } from "react";
import type { Config, ConfigManager } from "../../core/config";
import { ControlBar } from "../components/ControlBar";

interface MainMenuProps {
	config: Config;
	configManager: ConfigManager;
	onSelect: (option: string) => void;
}

const OPTIONS = [
	{ id: "update", label: "Update All" },
	{ id: "install", label: "Install Addon" },
	{ id: "manage", label: "Manage Addons" },
	{ id: "config", label: "Settings" },
] as const;

export const MainMenu: React.FC<MainMenuProps> = ({
	config,
	configManager,
	onSelect,
}) => {
	const { exit } = useApp();
	const [selectedIndex, setSelectedIndex] = useState(() => {
		const idx = OPTIONS.findIndex((opt) => opt.id === config.defaultMenuOption);
		return idx !== -1 ? idx : 0;
	});

	const [defaultOption, setDefaultOption] = useState(config.defaultMenuOption);
	const [feedbackMessage, setFeedbackMessage] = useState("");

	useInput((input, key) => {
		if (key.upArrow || input === "k") {
			setSelectedIndex((prev) => (prev > 0 ? prev - 1 : OPTIONS.length - 1));
		} else if (key.downArrow || input === "j") {
			setSelectedIndex((prev) => (prev < OPTIONS.length - 1 ? prev + 1 : 0));
		} else if (key.return) {
			const selected = OPTIONS[selectedIndex];
			if (selected) {
				onSelect(selected.id);
			}
		} else if (input === " " || key.rightArrow || input === "l") {
			// Set Default
			const selected = OPTIONS[selectedIndex];
			if (selected) {
				if (selected.id === "config") {
					setFeedbackMessage("Why would you even want that?");
					setTimeout(() => setFeedbackMessage(""), 2000);
				} else {
					const newDefault = selected.id as "update" | "manage" | "config";
					configManager.set("defaultMenuOption", newDefault);
					setDefaultOption(newDefault);
					setFeedbackMessage("Default Updated");
					setTimeout(() => setFeedbackMessage(""), 2000);
				}
			}
		} else if (input === "q" || key.escape) {
			exit();
		}
	});

	return (
		<Box flexDirection="column" gap={1}>
			{OPTIONS.map((opt, index) => {
				const isSelected = index === selectedIndex;
				const isDefault = opt.id === defaultOption;

				return (
					<Box key={opt.id}>
						<Text color={isSelected ? "green" : "white"}>
							{isSelected ? "> " : "  "}
							{opt.label}
						</Text>
						{isDefault && (
							<Box marginLeft={2}>
								<Text color="yellow">{" ●"}</Text>
							</Box>
						)}
					</Box>
				);
			})}
			<ControlBar
				message={
					feedbackMessage ? (
						<Text color="yellow">{feedbackMessage}</Text>
					) : undefined
				}
				controls={[
					{ key: "↑/↓", label: "nav" },
					{ key: "enter", label: "select" },
					{ key: "space", label: "set default" },
					{ key: "q", label: "quit" },
				]}
			/>
		</Box>
	);
};
