#!/usr/bin/env bun
import arg from "arg";
import { render } from "ink";

import { runCLI } from "./src/cli";
import { App } from "./src/tui/App";

async function main() {
	try {
		const args = arg({
			"--cli": Boolean,
			"--force": Boolean,
			"-f": "--force",
			"--dry-run": Boolean,
			"--test": Boolean,
			"--help": Boolean,
			"-h": "--help",
		}, { permissive: true });

		if (args["--help"]) {
			console.log(`
🍋 LemonUp - WoW Addon Manager

Usage:
  lemonup [flags]

Flags:
  --cli          Run in CLI mode (no TUI)
  --force, -f    Force update check/installation
  --dry-run      Simulate actions without modifying files
  --test         Run in test mode (uses temp directories)
  --help, -h     Show this help message
			`);
			process.exit(0);
		}

		if (args["--cli"]) {
			await runCLI();
		} else {
			// TUI Mode
			const { waitUntilExit } = render(
				<App
					force={args["--force"]}
					dryRun={args["--dry-run"]}
					testMode={args["--test"]}
				/>,
			);
			await waitUntilExit();
			process.exit(0);
		}
	} catch (err: any) {
		if (err.code === "ARG_UNKNOWN_OPTION") {
			console.error(err.message);
			console.error("Run 'lemonup --help' for usage.");
			process.exit(1);
		}
		throw err;
	}
}

main().catch(console.error);
