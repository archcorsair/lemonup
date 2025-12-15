import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Box, Text } from "ink";
import type React from "react";
import { useEffect, useState } from "react";
import { type Config, ConfigManager } from "../core/config";
import { AddonManager } from "../core/manager";
import { FirstRunWizard } from "./FirstRunWizard";
import { ConfigScreen } from "./screens/ConfigScreen";
import { MainMenu } from "./screens/MainMenu";
import { ManageScreen } from "./screens/ManageScreen";
import { UpdateScreen } from "./screens/UpdateScreen";

// Create a client
const queryClient = new QueryClient({
	defaultOptions: {
		queries: {
			// Cache for 1 minute by default (replacing our manual cache)
			staleTime: 60 * 1000,
			gcTime: 5 * 60 * 1000,
		},
	},
});

interface AppProps {
	force?: boolean;
	dryRun?: boolean;
	testMode?: boolean;
}

type Screen = "menu" | "update" | "manage" | "config";

export const App: React.FC<AppProps> = ({
	force = false,
	dryRun = false,
	testMode = false,
}) => {
	return (
		<QueryClientProvider client={queryClient}>
			<AppContent force={force} dryRun={dryRun} testMode={testMode} />
		</QueryClientProvider>
	);
};

const AppContent: React.FC<AppProps> = ({
	force = false,
	dryRun = false,
	testMode = false,
}) => {
	const [activeScreen, setActiveScreen] = useState<Screen>("menu");
	const [initialLoad, setInitialLoad] = useState(true);
	const [showWizard, setShowWizard] = useState(false);
	const [configManager, setConfigManager] = useState<ConfigManager | null>(
		null,
	);
	const [addonManager, setAddonManager] = useState<AddonManager | null>(null);
	const [config, setConfig] = useState<Config | null>(null);

	// Initialize Managers
	useEffect(() => {
		// Only initialize once
		if (configManager) return;

		let manager: ConfigManager;
		if (testMode) {
			// ... (keep test mode logic same as before but inside effect)
			const fs = require("node:fs");
			const path = require("node:path");
			const os = require("node:os");

			// 1. Setup Temp Dir
			const tempConfigDir = path.join(os.tmpdir(), "lemonup-test-config");
			// Don't wipe on start to allow persistence testing
			// if (fs.existsSync(tempConfigDir)) {
			// 	fs.rmSync(tempConfigDir, { recursive: true, force: true });
			// }
			fs.mkdirSync(tempConfigDir, { recursive: true });

			const configFile = path.join(tempConfigDir, "config.json");

			// 2. Read Config Source (Only if not already present)
			if (!fs.existsSync(configFile)) {
				let realConfig: Config | null = null;
				const sampleConfigPath = path.join(process.cwd(), "sample_config.json");

				if (fs.existsSync(sampleConfigPath)) {
					try {
						const raw = fs.readFileSync(sampleConfigPath, "utf-8");
						realConfig = JSON.parse(raw);
					} catch (e) {
						console.error("Failed to parse sample_config.json", e);
					}
				}

				if (!realConfig) {
					// Fallback to reading system config
					const realManager = new ConfigManager();
					if (realManager.hasConfigFile) {
						realConfig = realManager.get();
					}
				}

				// 3. Write to temp config
				if (realConfig) {
					fs.writeFileSync(configFile, JSON.stringify(realConfig));
				}
			}

			// 4. Return new Manager pointing to temp dir
			manager = new ConfigManager({
				cwd: tempConfigDir,
				overrides: {
					destDir: path.join(
						process.cwd(),
						"test-output",
						"Interface",
						"AddOns",
					),
				},
			});
		} else {
			manager = new ConfigManager();
		}

		setConfigManager(manager);
		setAddonManager(new AddonManager(manager));
		// Set config immediately if available
		if (manager.hasConfigFile) {
			setConfig(manager.get());
		}
	}, [testMode, configManager]);

	// Load Config
	useEffect(() => {
		if (!configManager || config) return; // ConfigManager not ready or config already loaded

		if (!configManager.hasConfigFile) {
			setShowWizard(true);
			return;
		}

		const cfg = configManager.get();
		setConfig(cfg);

		if (initialLoad) {
			// On first load, jump to default option?
			// User request: "I want Update Addons to be the default highlighted selection"
			// Wait, the user said "prompted with options that they can navigate to".
			// But they also said "Update Addons to be the default highlighted selection" on the menu.
			// It doesn't say "Skip menu and go to default".
			// So we should ALWAYS start at "menu", but the menu should highlight the default.
			// My MainMenu implementation handles highlighting the default.
			// So activeScreen should always start at "menu".
			setActiveScreen("menu");
			setInitialLoad(false);
		}
	}, [configManager, config, initialLoad]);

	const handleWizardComplete = () => {
		setShowWizard(false);
		// Trigger reload
		if (configManager) {
			const cfg = configManager.get();
			setConfig(cfg);
		}
	};

	// useInput hook removed as it was unused and empty

	if (showWizard && configManager) {
		return (
			<FirstRunWizard
				configManager={configManager}
				onComplete={handleWizardComplete}
			/>
		);
	}

	if (!config || !configManager) {
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
					🍋 LemonUp
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

			{activeScreen === "menu" && (
				<MainMenu
					config={config}
					configManager={configManager}
					onSelect={(option) => setActiveScreen(option as Screen)}
				/>
			)}

			{activeScreen === "update" && config && addonManager && (
				<UpdateScreen
					config={config}
					addonManager={addonManager}
					force={force}
					dryRun={dryRun}
					testMode={testMode}
					onBack={() => {
						// Refresh config on return to ensure UI is up to date
						setConfig(addonManager.getConfig());
						setActiveScreen("menu");
					}}
				/>
			)}

			{activeScreen === "manage" && config && addonManager && (
				<ManageScreen
					config={config}
					addonManager={addonManager}
					force={force}
					dryRun={dryRun}
					onBack={() => {
						// Refresh config on return
						setConfig(addonManager.getConfig());
						setActiveScreen("menu");
					}}
				/>
			)}

			{activeScreen === "config" && configManager && (
				<ConfigScreen
					configManager={configManager}
					onBack={() => setActiveScreen("menu")}
				/>
			)}
		</Box>
	);
};
