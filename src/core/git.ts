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
		return exitCode === 0;
	} catch (_error) {
		return false;
	}
}
