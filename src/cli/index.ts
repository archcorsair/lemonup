import arg from "arg";
import { ConfigManager } from "@/core/config";
import { AddonManager } from "@/core/manager";
import pkg from "../../package.json";

export async function runCLI() {
  const args = arg({
    "--force": Boolean,
    "-f": "--force",
    "--cli": Boolean, // Consumed by index.ts, but valid here
    "--version": Boolean,
    "-v": "--version",
  });

  if (args["--version"]) {
    console.log(pkg.version);
    return;
  }

  const force = args["--force"] || false;

  const configManager = new ConfigManager();
  const manager = new AddonManager(configManager);

  console.log(`Starting Lemonup v${pkg.version} (CLI Mode)...`);

  const addons = manager.getAllAddons();
  if (!addons.length) {
    console.warn("No addons found. Install addons first using the TUI.");
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
