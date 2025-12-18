import { Box, Text, useInput } from "ink";
import TextInput from "ink-text-input";
import type React from "react";
import { useEffect, useState } from "react";
import type { Config } from "../../core/config";
import type { AddonManager } from "../../core/manager";
import {
	getDefaultWoWPath,
	isPathConfigured,
	pathExists,
} from "../../core/paths";
import { ControlBar } from "../components/ControlBar";

interface InstallScreenProps {
	config: Config;
	addonManager: AddonManager;
	onBack: () => void;
}

type Mode =
	| "select"
	| "github-input"
	| "installing"
	| "result"
	| "config-auto-confirm"
	| "config-manual-input"
	| "confirm-reinstall";

export const InstallScreen: React.FC<InstallScreenProps> = ({
	config: initialConfig,
	addonManager,
	onBack,
}) => {
	// Local config state to reflect updates immediately
	const [config, setConfig] = useState(initialConfig);
	const [mode, setMode] = useState<Mode>("select");
	const [selection, setSelection] = useState(0);
	const [url, setUrl] = useState("");
	const [status, setStatus] = useState("");
	const [resultMessage, setResultMessage] = useState("");
	const [manualPath, setManualPath] = useState("");
	const [detectedPath, setDetectedPath] = useState("");

	// Store pending install action to retry after config
	const [pendingInstall, setPendingInstall] = useState<{
		type: "github" | "elvui";
		url?: string;
	} | null>(null);

	const [resultStatus, setResultStatus] = useState<"success" | "error">(
		"success",
	);

	const OPTIONS = [
		{ label: "Install from GitHub URL", action: "github", section: "General" },
		{ label: "Install ElvUI", action: "elvui", section: "TukUI" },
	];

	// Ensure we have the latest config
	useEffect(() => {
		setConfig(addonManager.getConfig());
	}, [addonManager]);

	const checkConfigAndInstall = async (
		type: "github" | "elvui",
		installUrl?: string,
	) => {
		// Safety Check: Duplicate Install
		let exists = false;
		if (type === "elvui") {
			exists = addonManager.isAlreadyInstalled("ElvUI");
		} else if (type === "github" && installUrl) {
			exists = addonManager.isAlreadyInstalled(installUrl);
		}

		if (exists) {
			setPendingInstall({ type, url: installUrl });
			setMode("confirm-reinstall");
			return;
		}

		if (!isPathConfigured(config.destDir)) {
			setPendingInstall({ type, url: installUrl });
			const def = getDefaultWoWPath();
			setDetectedPath(def);
			setMode("config-auto-confirm");
		} else {
			await handleInstall(type, installUrl);
		}
	};

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

	const savePathAndRetry = async (pathStr: string) => {
		try {
			addonManager.setConfigValue("destDir", pathStr);
			// Update local config
			setConfig({ ...config, destDir: pathStr });

			// Retry pending install
			if (pendingInstall) {
				await handleInstall(pendingInstall.type, pendingInstall.url);
			} else {
				setMode("select");
			}
		} catch (e) {
			setResultMessage(`Failed to save config: ${String(e)}`);
			setResultStatus("error");
			setMode("result");
		}
	};

	useInput(async (input, key) => {
		if (mode === "installing") return;

		if (mode === "result") {
			if (key.return || key.escape || input === "q") {
				setUrl("");
				setMode("select");
			}
			return;
		}

		if (mode === "config-auto-confirm") {
			if (input.toLowerCase() === "y" || key.return) {
				// User accepted auto-detected path
				await savePathAndRetry(detectedPath);
			} else if (input.toLowerCase() === "n" || key.escape) {
				// User rejected, offer manual input
				setMode("config-manual-input");
			}
			return;
		}

		if (mode === "confirm-reinstall") {
			if (input.toLowerCase() === "y") {
				// Proceed with install
				if (pendingInstall) {
					await handleInstall(pendingInstall.type, pendingInstall.url);
				}
			} else if (input.toLowerCase() === "n" || key.escape || key.return) {
				if (pendingInstall?.type === "github") {
					setMode("github-input");
					// Preserve the URL if available
					if (pendingInstall.url) setUrl(pendingInstall.url);
				} else {
					setMode("select");
				}
				setPendingInstall(null);
			}
			return;
		}

		if (mode === "config-manual-input") {
			if (key.return) {
				if (manualPath.trim()) {
					if (pathExists(manualPath.trim())) {
						await savePathAndRetry(manualPath.trim());
					} else {
						// Optional: show error or confirm creation?
						// For now, let's just accept it but warn?
						// Or simpler: just save it. The install command checks again.
						await savePathAndRetry(manualPath.trim());
					}
				}
			} else if (key.escape) {
				setMode("select"); // Cancel entire flow
				setPendingInstall(null);
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
					await checkConfigAndInstall("elvui");
				}
			} else if (key.escape || input === "q") {
				onBack();
			}
		} else if (mode === "github-input") {
			if (key.return) {
				if (url.trim()) {
					await checkConfigAndInstall("github", url);
				}
			} else if (key.escape) {
				setMode("select");
			}
		}
	});

	return (
		<Box flexDirection="column" gap={1}>
			<Text bold>Install Addon</Text>

			{mode === "config-auto-confirm" && (
				<Box
					flexDirection="column"
					borderColor="yellow"
					borderStyle="round"
					padding={1}
				>
					<Text color="yellow" bold>
						WoW Addon Directory Not Configured
					</Text>
					<Text>I detected a default location:</Text>
					<Text color="blue">{detectedPath}</Text>
					<Box marginTop={1}>
						<Text>Do you want to use this path? (Y/n)</Text>
					</Box>
				</Box>
			)}

			{mode === "confirm-reinstall" && (
				<Box
					flexDirection="column"
					borderColor="red"
					borderStyle="round"
					padding={1}
				>
					<Text color="red" bold>
						Warning: Addon Already Installed
					</Text>
					<Text>It looks like this addon is already installed.</Text>
					<Box marginTop={1}>
						<Text>Do you want to reinstall and overwrite it? (y/N)</Text>
					</Box>
				</Box>
			)}

			{mode === "config-manual-input" && (
				<Box
					flexDirection="column"
					borderColor="cyan"
					borderStyle="round"
					padding={1}
				>
					<Text bold>Enter WoW Addon Directory Path:</Text>
					<TextInput
						value={manualPath}
						onChange={setManualPath}
						onSubmit={() => {}}
					/>
					<Box marginTop={1}>
						<Text color="gray">Press Enter to save, Esc to cancel</Text>
					</Box>
				</Box>
			)}

			{mode === "select" && (
				<Box flexDirection="column">
					{(() => {
						let lastSection = "";
						return OPTIONS.map((opt, i) => {
							const showHeader = opt.section !== lastSection;
							lastSection = opt.section;
							return (
								<Box flexDirection="column" key={opt.action}>
									{showHeader && (
										<Box marginTop={i > 0 ? 1 : 0} marginBottom={0}>
											<Text color="magenta" bold underline>
												{opt.section}
											</Text>
										</Box>
									)}
									<Text color={i === selection ? "green" : "white"}>
										{i === selection ? "> " : "  "} {opt.label}
									</Text>
								</Box>
							);
						});
					})()}
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
					{ key: "esc", label: "back/cancel" },
					...(mode === "select" ? [{ key: "enter", label: "select" }] : []),
					...(mode === "github-input"
						? [{ key: "enter", label: "install" }]
						: []),
				]}
			/>
		</Box>
	);
};
