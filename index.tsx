#!/usr/bin/env bun
import arg from "arg";
import { render } from "ink";

import { runCLI } from "./src/cli";
import { App } from "./src/tui/App";

async function main() {
	// Simple flag check, we could use 'arg' here too but we want to know MODE first.
	const args = arg({
		"--cli": Boolean,
		"--force": Boolean,
		"-f": "--force",
		"--dry-run": Boolean,
	});

	if (args["--cli"]) {
		await runCLI();
	} else {
		// TUI Mode
		render(<App force={args["--force"]} dryRun={args["--dry-run"]} />);
	}
}

main().catch(console.error);
