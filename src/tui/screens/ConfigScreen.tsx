import { Box, Text, useInput } from "ink";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useState } from "react";
import type { ConfigManager } from "../../core/config";

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
	| "backupRetention";

export const ConfigScreen: React.FC<ScreenProps> = ({
	configManager,
	onBack,
}) => {
	const [maxConcurrent, setMaxConcurrent] = useState(3);
	const [destDir, setDestDir] = useState("");
	const [nerdFonts, setNerdFonts] = useState(true);
	const [checkInterval, setCheckInterval] = useState(60); // Display in seconds
	const [backupWTF, setBackupWTF] = useState(true);
	const [backupRetention, setBackupRetention] = useState(5);

	const [activeField, setActiveField] = useState<Field>("maxConcurrent");
	const [saved, setSaved] = useState(false);

	// Helpers for dynamic interval steps
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
	}, [configManager]);

	useInput((input, key) => {
		// Global Navigation
		if (key.upArrow) {
			const fields: Field[] = [
				"maxConcurrent",
				"destDir",
				"nerdFonts",
				"checkInterval",
				"backupWTF",
				"backupRetention",
			];
			const idx = fields.indexOf(activeField);
			const prev = fields[Math.max(0, idx - 1)];
			if (prev) setActiveField(prev);
			return;
		}
		if (key.downArrow || key.tab) {
			const fields: Field[] = [
				"maxConcurrent",
				"destDir",
				"nerdFonts",
				"checkInterval",
				"backupWTF",
				"backupRetention",
			];
			const idx = fields.indexOf(activeField);
			const next = fields[Math.min(fields.length - 1, idx + 1)];
			if (next) setActiveField(next);
			return;
		}

		if (key.escape) {
			onBack();
			return;
		}

		// Field Specific Handling
		if (activeField === "maxConcurrent") {
			if (key.leftArrow || input === "h") {
				const newVal = Math.max(1, maxConcurrent - 1);
				setMaxConcurrent(newVal);
				configManager.set("maxConcurrent", newVal);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
			if (key.rightArrow || input === "l") {
				const newVal = Math.min(10, maxConcurrent + 1);
				setMaxConcurrent(newVal);
				configManager.set("maxConcurrent", newVal);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
		}

		if (activeField === "destDir" && key.return) {
			configManager.set("destDir", destDir);
			setSaved(true);
			setTimeout(() => setSaved(false), 1000);
		}

		if (activeField === "nerdFonts") {
			if (
				key.leftArrow ||
				key.rightArrow ||
				input === "h" ||
				input === "l" ||
				input === " "
			) {
				setNerdFonts(!nerdFonts);
				configManager.set("nerdFonts", !nerdFonts);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
		}

		if (activeField === "checkInterval") {
			if (key.leftArrow || input === "h") {
				const newVal = getPrevInterval(checkInterval);
				setCheckInterval(newVal);
				configManager.set("checkInterval", newVal * 1000);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
			if (key.rightArrow || input === "l") {
				const newVal = getNextInterval(checkInterval);
				setCheckInterval(newVal);
				configManager.set("checkInterval", newVal * 1000);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
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
				setBackupWTF(!backupWTF);
				configManager.set("backupWTF", !backupWTF);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
		}

		if (activeField === "backupRetention") {
			if (key.leftArrow || input === "h") {
				const newVal = Math.max(1, backupRetention - 1);
				setBackupRetention(newVal);
				configManager.set("backupRetention", newVal);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
			if (key.rightArrow || input === "l") {
				const newVal = Math.min(20, backupRetention + 1); // Cap at 20
				setBackupRetention(newVal);
				configManager.set("backupRetention", newVal);
				setSaved(true);
				setTimeout(() => setSaved(false), 1000);
			}
		}
	});

	const renderLabel = (field: Field, label: string) => (
		<Box
			borderStyle={activeField === field ? "round" : undefined}
			borderColor="blue"
			paddingX={1}
			width="100%"
		>
			<Box width={30}>
				<Text bold>{label}: </Text>
			</Box>
			<Box flexGrow={1}>
				{field === "maxConcurrent" && (
					<Text color="yellow" bold>
						{"<"} {maxConcurrent} {">"}
					</Text>
				)}
				{field === "destDir" &&
					(activeField === "destDir" ? (
						<TextInput
							value={destDir}
							onChange={setDestDir}
							onSubmit={(val) => {
								configManager.set("destDir", val);
								setSaved(true);
								setTimeout(() => setSaved(false), 1000);
							}}
						/>
					) : (
						<Text color={destDir ? "white" : "gray"}>
							{destDir || "Not Configured"}
						</Text>
					))}
				{field === "nerdFonts" && (
					<Text color={nerdFonts ? "green" : "red"}>
						{nerdFonts ? "Enabled" : "Disabled"}
					</Text>
				)}
				{field === "checkInterval" && (
					<Text color="yellow">
						{"<"} {formatInterval(checkInterval)} {">"}
					</Text>
				)}
				{field === "backupWTF" && (
					<Text color={backupWTF ? "green" : "red"}>
						{backupWTF ? "Enabled" : "Disabled"}
					</Text>
				)}
				{field === "backupRetention" && (
					<Text color="yellow" bold>
						{"<"} {backupRetention} {">"}
					</Text>
				)}
			</Box>
		</Box>
	);

	return (
		<Box flexDirection="column" padding={1}>
			<Text color="blue" bold>
				Configuration
			</Text>

			<Box marginTop={1} flexDirection="column" gap={0}>
				{renderLabel("maxConcurrent", "Max Concurrent Downloads")}
				{renderLabel("destDir", "WoW Interface Directory")}
				{renderLabel("nerdFonts", "Nerd Fonts (Icons)")}
				{renderLabel("checkInterval", "Update Check Interval")}
				{renderLabel("backupWTF", "Backup 'WTF' Folder")}
				{renderLabel("backupRetention", "Backup Retention Count")}

				<Box marginLeft={2} height={1} marginTop={1}>
					{saved && <Text color="green">Saved!</Text>}
				</Box>
			</Box>

			<Box marginTop={0}>
				<Text color="gray">
					{activeField === "destDir"
						? "Type path and press Enter to save."
						: "Use Left/Right arrows to change."}
					{" Up/Down to switch."}
				</Text>
			</Box>
		</Box>
	);
};
