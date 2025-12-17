import { spawn } from "bun";

async function run() {
	const remoteUrl = "https://github.com/tukui-org/ElvUI";
	const branch = "main";

	console.log(`Checking remote: ${remoteUrl} (${branch})`);

	const cmd = ["git", "ls-remote", remoteUrl, `refs/heads/${branch}`];
	const proc = Bun.spawn(cmd, { stdout: "pipe" });
	const output = await new Response(proc.stdout).text();

	const remoteHash = output.split("\t")[0];
	console.log(`Remote Hash (HEAD): ${remoteHash}`);

	// Simulate local version from previous debugging
	const localVersion = "v14.04-85-gcd34244";
	const hashMatch = localVersion.match(/-g([a-f0-9]+)/);
	const localHash = hashMatch ? hashMatch[1] : "null";
	console.log(`Local Version: ${localVersion}`);
	console.log(`Extracted Local Hash: ${localHash}`);

	if (remoteHash && localHash) {
		const match = remoteHash.startsWith(localHash);
		console.log(`Match? ${match}`);
		if (!match) {
			console.log("MISMATCH! The zip file is likely behind the git repo.");
		}
	}
}

run();
