import fsp from "node:fs/promises";
import path from "node:path";
import yauzl from "yauzl-promise";

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
 * Unzips a file to a destination directory using `yauzl-promise`.
 */
export async function unzip(
	zipPath: string,
	destDir: string,
): Promise<boolean> {
	let zipFile: any;
	try {
		zipFile = await yauzl.open(zipPath);
		try {
			for await (const entry of zipFile) {
				const entryPath = path.join(destDir, entry.filename);

				if (entry.filename.endsWith("/")) {
					// Directory
					await fsp.mkdir(entryPath, { recursive: true });
				} else {
					// File
					const readStream = await entry.openReadStream();
					const parentDir = path.dirname(entryPath);
					await fsp.mkdir(parentDir, { recursive: true });

					// Use Bun.write which consumes the readable stream directly
					await Bun.write(entryPath, readStream);
				}
			}
		} finally {
			await zipFile.close();
		}
		return true;
	} catch (error) {
		throw new Error(
			`Unzip execution failed: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}
