import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import AdmZip from "adm-zip";
import { logger } from "./logger";

export async function download(
	url: string,
	destPath: string,
): Promise<boolean> {
	try {
		logger.log("Downloader", `Downloading: ${url}`);
		const response = await fetch(url, {
			headers: {
				"User-Agent":
					"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
			},
		});

		if (!response.ok) {
			logger.error(
				"Downloader",
				`Download failed. Status: ${response.status} ${response.statusText} for ${url}`,
			);
			return false;
		}

		await Bun.write(destPath, response);
		logger.log("Downloader", "Download complete");
		return true;
	} catch (error) {
		logger.error("Downloader", `Download threw error for ${url}`, error);
		return false;
	}
}

export async function unzip(
	zipPath: string,
	destDir: string,
): Promise<boolean> {
	try {
		// AdmZip is synchronous by default, which is fine for this use case
		// and avoids the native binding complexity.
		const zip = new AdmZip(zipPath);
		const zipEntries = zip.getEntries();

		for (const entry of zipEntries) {
			// Validate entry path to prevent directory traversal
			if (entry.entryName.includes("..")) {
				logger.error("Downloader", `Skipping unsafe entry: ${entry.entryName}`);
				continue;
			}

			const entryPath = path.resolve(destDir, entry.entryName);
			// Ensure entryPath is within destDir
			if (!entryPath.startsWith(destDir)) {
				logger.error(
					"Downloader",
					`Skipping entry outside destDir: ${entry.entryName}`,
				);
				continue;
			}

			if (entry.isDirectory) {
				await fsp.mkdir(entryPath, { recursive: true });
			} else {
				const parentDir = path.dirname(entryPath);
				await fsp.mkdir(parentDir, { recursive: true });
				
				// AdmZip synchronous extraction
				const data = entry.getData();
				await Bun.write(entryPath, data);
			}
		}

		return true;
	} catch (error) {
		throw new Error(
			`Unzip execution failed: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}
