/**
 * Downloads a file from a URL to a local destination using Bun's native fetch and write.
 */
export async function download(
	url: string,
	destPath: string,
): Promise<boolean> {
	try {
		const response = await fetch(url, {
			headers: {
				"User-Agent":
					"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
			},
		});

		if (!response.ok) {
			return false;
		}

		await Bun.write(destPath, response);
		return true;
	} catch (_error) {
		return false;
	}
}

/**
 * Unzips a file to a destination directory using the system `unzip` command.
 * Equivalent to: unzip -q -o <zipPath> -d <destDir>
 */
export async function unzip(
	zipPath: string,
	destDir: string,
): Promise<boolean> {
	try {
		// Using Bun shell for easier command execution if available, or Bun.spawn
		// Since we used spawn in GitClient, let's use Bun.spawn here too for consistency,
		// or just Bun.spawnSync if we want to block (but async is better).
		// Actually, for simple commands, `Bun.$` is nicer but let's stick to spawn for now or specific command.

		const proc = Bun.spawn(["unzip", "-q", "-o", zipPath, "-d", destDir], {
			stdout: "ignore",
			stderr: "ignore",
		});
		const exitCode = await proc.exited;
		return exitCode === 0;
	} catch (_error) {
		return false;
	}
}
