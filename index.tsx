#!/usr/bin/env bun
import arg from "arg";
import { render } from "ink";
import { runCLI } from "@/cli";
import { App } from "@/tui/App";
import pkg from "./package.json";

async function main() {
	try {
		const args = arg(
			{
				"--cli": Boolean,
				"--force": Boolean,
				"-f": "--force",
				"--dry-run": Boolean,
				"--test": Boolean,
				"--help": Boolean,
				"-h": "--help",
				"--version": Boolean,
				"-v": "--version",
			},
			{ permissive: true },
		);

		if (args["--version"]) {
			console.log(pkg.version);
			process.exit(0);
		}

		if (args["--help"]) {
			console.log(`
🍋 LemonUp - WoW Addon Manager (v${pkg.version})

Usage:
  lemonup [flags]

Flags:
  --cli          Run in CLI mode (no TUI)
  --force, -f    Force update check/installation
  --dry-run      Simulate actions without modifying files
  --test         Run in test mode (uses temp directories)
  --version, -v  Show version number
  --help, -h     Show this help message
			`);
			process.exit(0);
		}

		if (args["--cli"]) {
			await runCLI();
		} else {
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
	} catch (err) {
		// biome-ignore lint/suspicious/noExplicitAny: error code property is not standard but present on ARG_UNKNOWN_OPTION
		if (err instanceof Error && (err as any).code === "ARG_UNKNOWN_OPTION") {
			console.error(err.message);
			console.error("Run 'lemonup --help' for usage.");
			process.exit(1);
		}

		if (err instanceof Error) {
			console.error(err.message);
		} else {
			console.error(err);
		}
		process.exit(1);
	}
}

main().catch(console.error);
