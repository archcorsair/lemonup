import arg from "arg";
import { ConfigManager } from "../core/config";
import { AddonManager } from "../core/manager";

export async function runCLI() {
	const args = arg({
		"--force": Boolean,
		"-f": "--force",
		"--cli": Boolean, // Consumed by index.ts, but valid here
	});

	const force = args["--force"] || false;

	const configManager = new ConfigManager();
	const manager = new AddonManager(configManager);

	console.log("Starting Lemonup (CLI Mode)...");

	const config = manager.getConfig();
	if (!config.repositories.length) {
		console.warn(
			"No repositories found in config. Please verify configuration at:",
			configManager.path,
		);
		// Don't exit here, just warn.
	}

	try {
		const results = await manager.updateAll(force);
		let successCount = 0;
		let failCount = 0;

		for (const res of results) {
			if (res.success) {
				if (res.updated) {
					console.log(`[UPDATED] ${res.repoName} - ${res.message}`);
				} else {
					console.log(
						`[OK]      ${res.repoName} - ${res.message || "Up to date"}`,
					);
				}
				successCount++;
			} else {
				console.error(`[ERROR]   ${res.repoName} - ${res.error}`);
				failCount++;
			}
		}

		console.log(`\nFinished: ${successCount} successful, ${failCount} failed.`);

		if (failCount > 0) {
			process.exit(1);
		}
	} catch (error) {
		console.error("Fatal error during update process:", error);
		process.exit(1);
	}
}
