import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Box, Text } from "ink";
import Spinner from "ink-spinner";
import type React from "react";
import { useEffect, useState } from "react";
import { type Config, ConfigManager } from "@/core/config";
import { AddonManager } from "@/core/manager";
import pkg from "../../package.json";
import { FirstRunWizard } from "./FirstRunWizard";
import { ConfigScreen } from "./screens/ConfigScreen";
import { InstallScreen } from "./screens/InstallScreen";
import { MainMenu } from "./screens/MainMenu";
import { ManageScreen } from "./screens/ManageScreen";
import { UpdateScreen } from "./screens/UpdateScreen";
import { useAppStore } from "./store/useAppStore";

const queryClient = new QueryClient({
	defaultOptions: {
		queries: {
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

// Workaround for React 19 + Ink type mismatch
const SpinnerFixed = Spinner as unknown as React.FC<{
	type?: string;
}>;

const AppContent: React.FC<AppProps> = ({
	force = false,
	dryRun = false,
	testMode = false,
}) => {
	const activeScreen = useAppStore((state) => state.activeScreen);
	const isBusy = useAppStore((state) => state.isBusy);
	const lastMenuSelection = useAppStore((state) => state.lastMenuSelection);
	const navigate = useAppStore((state) => state.navigate);
	const setLastMenuSelection = useAppStore(
		(state) => state.setLastMenuSelection,
	);

	const [initialLoad, setInitialLoad] = useState(true);
	const [showWizard, setShowWizard] = useState(false);
	const [configManager, setConfigManager] = useState<ConfigManager | null>(
		null,
	);
	const [addonManager, setAddonManager] = useState<AddonManager | null>(null);
	const [config, setConfig] = useState<Config | null>(null);

	useEffect(() => {
		if (configManager) return;

		let manager: ConfigManager;
		if (testMode) {
			const fs = require("node:fs");
			const path = require("node:path");
			const os = require("node:os");

			const tempConfigDir = path.join(os.tmpdir(), "lemonup-test-config");
			fs.mkdirSync(tempConfigDir, { recursive: true });

			const configFile = path.join(tempConfigDir, "config.json");

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
					const realManager = new ConfigManager();
					if (realManager.hasConfigFile) {
						realConfig = realManager.get();
					}
				}

				if (realConfig) {
					fs.writeFileSync(configFile, JSON.stringify(realConfig));
				}
			}

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
		if (manager.hasConfigFile) {
			setConfig(manager.get());
		}
	}, [testMode, configManager]);

	useEffect(() => {
		if (!configManager || config) return;

		if (!configManager.hasConfigFile) {
			setShowWizard(true);
			return;
		}

		const cfg = configManager.get();
		setConfig(cfg);

		if (initialLoad) {
			navigate("menu");
			setInitialLoad(false);
		}
	}, [configManager, config, initialLoad, navigate]);

	const handleWizardComplete = () => {
		setShowWizard(false);
		if (configManager) {
			const cfg = configManager.get();
			setConfig(cfg);
		}
	};

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
					LemonUp 🍋
				</Text>
				<Box marginLeft={1}>
					<Text color="gray">v{pkg.version}</Text>
				</Box>
				{dryRun && (
					<Box marginLeft={2}>
						<Text color="yellow" bold>
							[DRY RUN]
						</Text>
					</Box>
				)}
				{isBusy && (
					<Box marginLeft={2}>
						<Text color="yellow">
							<SpinnerFixed type="dots" /> Working...
						</Text>
					</Box>
				)}
			</Box>

			{activeScreen === "menu" && (
				<MainMenu
					config={config}
					configManager={configManager}
					initialSelection={lastMenuSelection}
					onSelect={(option) => {
						setLastMenuSelection(option);
						// @ts-expect-error: Screen type is broad
						navigate(option);
					}}
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
						setConfig(addonManager.getConfig());
						navigate("menu");
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
						setConfig(addonManager.getConfig());
						navigate("menu");
					}}
				/>
			)}

			{activeScreen === "install" && config && addonManager && (
				<InstallScreen
					config={config}
					addonManager={addonManager}
					onBack={() => {
						setConfig(addonManager.getConfig());
						navigate("menu");
					}}
				/>
			)}

			{activeScreen === "config" && configManager && (
				<ConfigScreen
					configManager={configManager}
					onBack={() => navigate("menu")}
				/>
			)}
		</Box>
	);
};
