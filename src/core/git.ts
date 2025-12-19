/**
 * Fetches the latest commit hash from a remote git repository.
 * Equivalent to: git ls-remote <url> refs/heads/<branch>
 */
export async function getRemoteCommit(
	remoteUrl: string,
	branch = "main",
): Promise<string | null> {
	const cmd = ["git", "ls-remote", remoteUrl, `refs/heads/${branch}`];
	try {
		const proc = Bun.spawn(cmd, {
			stdout: "pipe",
			stderr: "pipe", // Capture stderr to avoid polluting console on error
		});
		const output = await new Response(proc.stdout).text();
		const exitCode = await proc.exited;

		if (exitCode !== 0) {
			// TODO: Log error matching Nushell's behavior?
			return null;
		}

		if (!output.trim()) {
			return null;
		}

		// Output format: <hash>\trefs/heads/<branch>
		const match = output.match(/^([a-f0-9]+)\s/);
		return match ? (match[1] ?? null) : null;
	} catch (_error) {
		// TODO: Log error
		return null;
	}
}

/**
 * Clones a git repository to a specific path.
 * Equivalent to: git clone --quiet --depth 1 --branch <branch> <url> <path>
 */
export async function clone(
	remoteUrl: string,
	branch: string,
	destPath: string,
): Promise<boolean> {
	const cmd = [
		"git",
		"clone",
		"--quiet",
		"--depth",
		"1",
		"--branch",
		branch,
		remoteUrl,
		destPath,
	];

	try {
		const proc = Bun.spawn(cmd, {
			stdout: "ignore",
			stderr: "pipe",
		});
		const exitCode = await proc.exited;
		if (exitCode !== 0) {
			const stderr = await new Response(proc.stderr).text();
			throw new Error(`Git exited with code ${exitCode}: ${stderr.trim()}`);
		}
		return true;
	} catch (error) {
		throw new Error(
			`Git clone execution failed: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}

/**
 * Gets the current commit hash of a local git repository.
 * Equivalent to: git rev-parse HEAD
 */
export async function getCurrentCommit(
	cwd: string,
	gitDir?: string, // Optional, but usually implicit in cwd
): Promise<string | null> {
	const cmd = ["git", "rev-parse", "HEAD"];
	try {
		const proc = Bun.spawn(cmd, {
			cwd,
			stdout: "pipe",
			stderr: "pipe",
		});
		const output = await new Response(proc.stdout).text();
		const exitCode = await proc.exited;

		if (exitCode !== 0) {
			return null;
		}

		return output.trim() || null;
	} catch (_error) {
		return null;
	}
}

/**
 * Fetches the latest tag from a remote git repository.
 * Returns the tag name (e.g. "1.2.3" or "v1.2.3").
 */
export async function getLatestTag(remoteUrl: string): Promise<string | null> {
	const cmd = ["git", "ls-remote", "--tags", "--sort=-v:refname", remoteUrl];
	try {
		const proc = Bun.spawn(cmd, {
			stdout: "pipe",
			stderr: "pipe",
		});
		const output = await new Response(proc.stdout).text();
		const exitCode = await proc.exited;

		if (exitCode !== 0) return null;

		// Output lines like: hash\trefs/tags/v1.0.0
		const lines = output.trim().split("\n");
		for (const line of lines) {
			const match = line.match(/refs\/tags\/([^^{}]+)$/);
			if (match?.[1]) {
				// Return the first one (sorted by version descending by git)
				return match[1];
			}
		}
		return null;
	} catch (_error) {
		return null;
	}
}
