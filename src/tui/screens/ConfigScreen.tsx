import { Box, Text, useInput } from "ink";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useState } from "react";
import type { ConfigManager } from "@/core/config";
import { ControlBar } from "@/tui/components/ControlBar";
import { useToast } from "@/tui/hooks/useToast";
import { useAppStore } from "@/tui/store/useAppStore";

interface ScreenProps {
	configManager: ConfigManager;
	onBack: () => void;
}

type Field =
	| "maxConcurrent"
	| "destDir"
	| "nerdFonts"
	| "checkInterval"
	| "backupWTF"
	| "backupRetention"
	| "debug";

export const ConfigScreen: React.FC<ScreenProps> = ({
	configManager,
	onBack,
}) => {
	const flashKey = useAppStore((state) => state.flashKey);
	const [maxConcurrent, setMaxConcurrent] = useState(3);
	const [destDir, setDestDir] = useState("");
	const [nerdFonts, setNerdFonts] = useState(true);
	const [checkInterval, setCheckInterval] = useState(60); // Display in seconds
	const [backupWTF, setBackupWTF] = useState(true);
	const [backupRetention, setBackupRetention] = useState(5);
	const [debug, setDebug] = useState(false);
	const [commandHelp, setCommandHelp] = useState<string | null>(null);

	const [activeField, setActiveField] = useState<Field>("maxConcurrent");
	const { toast, showToast } = useToast();

	const getNextInterval = (current: number) => {
		if (current < 60) return Math.min(60, current + 10);
		if (current < 900) return Math.min(900, current + 60); // 1m steps up to 15m
		return Math.min(3600, current + 300); // 5m steps up to 60m
	};

	const getPrevInterval = (current: number) => {
		if (current <= 60) return Math.max(0, current - 10);
		if (current <= 900) return Math.max(60, current - 60);
		return Math.max(900, current - 300);
	};

	const formatInterval = (seconds: number) => {
		if (seconds < 60) return `${seconds}s`;
		const mins = Math.floor(seconds / 60);
		const secs = seconds % 60;
		if (secs === 0) return `${mins}m`;
		return `${mins}m ${secs}s`;
	};

	useEffect(() => {
		const cfg = configManager.get();
		setMaxConcurrent(cfg.maxConcurrent);
		setDestDir(cfg.destDir === "NOT_CONFIGURED" ? "" : cfg.destDir);
		setNerdFonts(cfg.nerdFonts);
		setCheckInterval(cfg.checkInterval / 1000); // Storage is ms, UI is seconds
		setBackupWTF(cfg.backupWTF);
		setBackupRetention(cfg.backupRetention);
		setDebug(cfg.debug);
	}, [configManager]);

	useInput((input, key) => {
		const fields: Field[] = [
			"maxConcurrent",
			"destDir",
			"nerdFonts",
			"checkInterval",
			"backupWTF",
			"backupRetention",
			"debug",
		];

		if (key.upArrow) {
			flashKey("↑/↓");
			const idx = fields.indexOf(activeField);
			const prev = fields[Math.max(0, idx - 1)];
			if (prev) setActiveField(prev);
			return;
		}
		if (key.downArrow || key.tab) {
			flashKey("↑/↓");
			const idx = fields.indexOf(activeField);
			const next = fields[Math.min(fields.length - 1, idx + 1)];
			if (next) setActiveField(next);
			return;
		}

		if (key.escape) {
			flashKey("esc");
			onBack();
			return;
		}

		if (activeField === "maxConcurrent") {
			if (key.leftArrow || input === "h") {
				flashKey("←/→");
				const newVal = Math.max(1, maxConcurrent - 1);
				setMaxConcurrent(newVal);
				configManager.set("maxConcurrent", newVal);
				showToast("Saved!", 1000);
			}
			if (key.rightArrow || input === "l") {
				flashKey("←/→");
				const newVal = Math.min(10, maxConcurrent + 1);
				setMaxConcurrent(newVal);
				configManager.set("maxConcurrent", newVal);
				showToast("Saved!", 1000);
			}
		}

		if (activeField === "destDir" && key.return) {
			flashKey("enter");
			configManager.set("destDir", destDir);
			showToast("Saved!", 1000);
		}

		if (activeField === "nerdFonts") {
			if (
				key.leftArrow ||
				key.rightArrow ||
				input === "h" ||
				input === "l" ||
				input === " "
			) {
				flashKey(input === " " ? "space" : "←/→");
				setNerdFonts(!nerdFonts);
				configManager.set("nerdFonts", !nerdFonts);
				showToast("Saved!", 1000);
			}
		}

		if (activeField === "checkInterval") {
			if (key.leftArrow || input === "h") {
				flashKey("←/→");
				const newVal = getPrevInterval(checkInterval);
				setCheckInterval(newVal);
				configManager.set("checkInterval", newVal * 1000);
				showToast("Saved!", 1000);
			}
			if (key.rightArrow || input === "l") {
				flashKey("←/→");
				const newVal = getNextInterval(checkInterval);
				setCheckInterval(newVal);
				configManager.set("checkInterval", newVal * 1000);
				showToast("Saved!", 1000);
			}
		}

		if (activeField === "backupWTF") {
			if (
				key.leftArrow ||
				key.rightArrow ||
				input === "h" ||
				input === "l" ||
				input === " "
			) {
				flashKey(input === " " ? "space" : "←/→");
				setBackupWTF(!backupWTF);
				configManager.set("backupWTF", !backupWTF);
				showToast("Saved!", 1000);
			}
		}

		if (activeField === "backupRetention") {
			if (key.leftArrow || input === "h") {
				flashKey("←/→");
				const newVal = Math.max(1, backupRetention - 1);
				setBackupRetention(newVal);
				configManager.set("backupRetention", newVal);
				showToast("Saved!", 1000);
			}
			if (key.rightArrow || input === "l") {
				flashKey("←/→");
				const newVal = Math.min(20, backupRetention + 1); // Cap at 20
				setBackupRetention(newVal);
				configManager.set("backupRetention", newVal);
				showToast("Saved!", 1000);
			}
		}

		if (activeField === "debug") {
			if (
				key.leftArrow ||
				key.rightArrow ||
				input === "h" ||
				input === "l" ||
				input === " "
			) {
				flashKey(input === " " ? "space" : "←/→");
				setDebug(!debug);
				configManager.set("debug", !debug);
				showToast("Saved!", 1000);
			}
		}
	});

	const ConfigOption: React.FC<{
		label: string;
		isActive: boolean;
		helpText?: string;
		children: React.ReactNode;
	}> = ({ label, isActive, helpText, children }) => {
		useEffect(() => {
			if (isActive) {
				setCommandHelp(
					helpText || "Use Left/Right arrows to change. Up/Down to switch.",
				);
			}
		}, [isActive, helpText]);

		return (
			<Box
				borderStyle={isActive ? "round" : undefined}
				borderColor="blue"
				paddingX={1}
				width="100%"
			>
				<Box width={30}>
					<Text bold>{label}: </Text>
				</Box>
				<Box flexGrow={1}>{children}</Box>
			</Box>
		);
	};

	return (
		<Box flexDirection="column" padding={1}>
			<Text color="blue" bold>
				Configuration
			</Text>

			<Box marginTop={1} flexDirection="column" gap={0}>
				<ConfigOption
					label="Max Concurrent Downloads"
					isActive={activeField === "maxConcurrent"}
				>
					<Text color="yellow" bold>
						{"<"} {maxConcurrent} {">"}
					</Text>
				</ConfigOption>

				<ConfigOption
					label="WoW Interface Directory"
					isActive={activeField === "destDir"}
					helpText="Type path and press Enter to save."
				>
					{activeField === "destDir" ? (
						<TextInput
							value={destDir}
							onChange={setDestDir}
							onSubmit={(val) => {
								configManager.set("destDir", val);
								showToast("Saved!", 1000);
							}}
						/>
					) : (
						<Text color={destDir ? "white" : "gray"}>
							{destDir || "Not Configured"}
						</Text>
					)}
				</ConfigOption>

				<ConfigOption
					label="Nerd Fonts (Icons)"
					isActive={activeField === "nerdFonts"}
				>
					<Text color={nerdFonts ? "green" : "red"}>
						{nerdFonts ? "Enabled" : "Disabled"}
					</Text>
				</ConfigOption>

				<ConfigOption
					label="Update Check Interval"
					isActive={activeField === "checkInterval"}
				>
					<Text color="yellow">
						{"<"} {formatInterval(checkInterval)} {">"}
					</Text>
				</ConfigOption>

				<ConfigOption
					label="Backup 'WTF' Folder"
					isActive={activeField === "backupWTF"}
				>
					<Text color={backupWTF ? "green" : "red"}>
						{backupWTF ? "Enabled" : "Disabled"}
					</Text>
				</ConfigOption>

				<ConfigOption
					label="Backup Retention Count"
					isActive={activeField === "backupRetention"}
				>
					<Text color="yellow" bold>
						{"<"} {backupRetention} {">"}
					</Text>
				</ConfigOption>

				<ConfigOption
					label="Enable Debug Logging"
					isActive={activeField === "debug"}
				>
					<Text color={debug ? "green" : "gray"}>
						{debug ? "Enabled" : "Disabled"}
					</Text>
				</ConfigOption>
			</Box>

			<ControlBar
				message={
					toast?.message ? (
						<Text color="green">{toast.message}</Text>
					) : commandHelp ? (
						<Text>{commandHelp}</Text>
					) : undefined
				}
				controls={[
					{ key: "↑/↓", label: "nav" },
					{ key: "←/→", label: "modify" },
					{ key: "space", label: "toggle" },
					{ key: "enter", label: "save text" },
					{ key: "esc", label: "back" },
				]}
			/>
		</Box>
	);
};
