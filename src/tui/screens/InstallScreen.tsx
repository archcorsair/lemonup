import { Box, Text, useApp, useInput } from "ink";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useState } from "react";
import type { Config } from "../../core/config";
import type { AddonManager } from "../../core/manager";
import { ControlBar } from "../components/ControlBar";

interface InstallScreenProps {
	config: Config;
	addonManager: AddonManager;
	onBack: () => void;
}

type Mode = "select" | "github-input" | "installing" | "result";

export const InstallScreen: React.FC<InstallScreenProps> = ({
	config,
	addonManager,
	onBack,
}) => {
	const [mode, setMode] = useState<Mode>("select");
	const [selection, setSelection] = useState(0);
	const [url, setUrl] = useState("");
	const [status, setStatus] = useState("");
	const [resultMessage, setResultMessage] = useState("");

	const [resultStatus, setResultStatus] = useState<"success" | "error">(
		"success",
	);

	const OPTIONS = [
		{ label: "Install from GitHub URL", action: "github" },
		{ label: "Install ElvUI", action: "elvui" },
	];

	const handleInstall = async (
		type: "github" | "elvui",
		installUrl?: string,
	) => {
		setMode("installing");
		setStatus("Installing...");
		setResultStatus("success");

		try {
			if (type === "github") {
				if (!installUrl) throw new Error("No URL provided");
				setStatus(`Cloning ${installUrl}...`);
				const res = await addonManager.installFromUrl(installUrl);
				if (res.success) {
					setResultMessage(
						`Successfully installed: ${res.installedAddons.join(", ")}`,
					);
					setResultStatus("success");
				} else {
					setResultMessage(res.error || "Unknown Error");
					setResultStatus("error");
				}
			} else if (type === "elvui") {
				setStatus("Downloading ElvUI from TukUI...");
				await addonManager.installTukUI(
					"https://api.tukui.org/v1/download/dev/elvui/main",
					"ElvUI",
					["ElvUI_Options", "ElvUI_Libraries"],
				);

				setResultMessage("ElvUI Installed Successfully");
				setResultStatus("success");
			}
		} catch (e) {
			setResultMessage(e instanceof Error ? e.message : String(e));
			setResultStatus("error");
		}
		setMode("result");
	};

	useInput((input, key) => {
		if (mode === "installing") return;

		if (mode === "result") {
			if (key.return || key.escape || input === "q") {
				setUrl("");
				setMode("select");
			}
			return;
		}

		if (mode === "select") {
			if (key.upArrow) {
				setSelection((prev) => (prev > 0 ? prev - 1 : prev));
			} else if (key.downArrow) {
				setSelection((prev) => (prev < OPTIONS.length - 1 ? prev + 1 : prev));
			} else if (key.return) {
				const action = OPTIONS[selection]?.action;
				if (action === "github") {
					setMode("github-input");
				} else if (action === "elvui") {
					handleInstall("elvui");
				}
			} else if (key.escape || input === "q") {
				onBack();
			}
		} else if (mode === "github-input") {
			if (key.return) {
				if (url.trim()) {
					handleInstall("github", url);
				}
			} else if (key.escape) {
				setMode("select");
			}
		}
	});

	return (
		<Box flexDirection="column" gap={1}>
			<Text bold>Install Addon</Text>

			{mode === "select" && (
				<Box flexDirection="column">
					{OPTIONS.map((opt, i) => (
						<Text key={opt.action} color={i === selection ? "green" : "white"}>
							{i === selection ? "> " : "  "} {opt.label}
						</Text>
					))}
				</Box>
			)}

			{mode === "github-input" && (
				<Box>
					<Text>Repo URL: </Text>
					<TextInput value={url} onChange={setUrl} onSubmit={() => {}} />
				</Box>
			)}

			{mode === "installing" && <Text color="yellow">{status}</Text>}

			{mode === "result" && (
				<Box flexDirection="column">
					<Text color={resultStatus === "success" ? "green" : "red"}>
						{resultStatus === "success" ? "✔ " : "✘ "}
						{resultMessage}
					</Text>
					<Text color="gray">Press Enter to continue</Text>
				</Box>
			)}

			<ControlBar
				message={
					mode === "github-input" ? <Text>Enter Git URL</Text> : undefined
				}
				controls={[
					{ key: "esc", label: "back" },
					...(mode === "select" ? [{ key: "enter", label: "select" }] : []),
					...(mode === "github-input"
						? [{ key: "enter", label: "install" }]
						: []),
				]}
			/>
		</Box>
	);
};
