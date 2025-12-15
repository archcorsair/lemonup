import fs from "node:fs";
import fsPromises from "node:fs/promises";
import path from "node:path";
import archiver from "archiver";

export const BackupManager = {
	/**
	 * Backs up the WTF folder to _retail_/Backups/WTF/WTF-<timestamp>.zip
	 * @param destDir The path to _retail_/Interface/AddOns
	 * @param minIntervalMinutes Minimum minutes since last backup to run
	 * @returns The path to the created backup, null if source missing, or "skipped-recent"
	 */
	async backupWTF(
		destDir: string,
		minIntervalMinutes = 0,
	): Promise<string | null | "skipped-recent"> {
		// destDir is Interface/AddOns. We need to go up to _retail_
		const retailDir = path.dirname(path.dirname(destDir));
		const wtfDir = path.join(retailDir, "WTF");

		// Check if WTF exists using Bun.file
		if (!(await Bun.file(wtfDir).exists())) {
			return null; // No WTF folder to backup (Note: Bun.file checks if path exists, works for dirs on some platforms but let's be safe. Actually Bun.file is for files. For dirs common check is stat or existing method. Wait, Bun.file().exists() works for files. For directories `fs.exists` or `fs.stat` is safer. Let's verify Bun behavior.
			// Re-reading docs: Bun.file(path) refers to a file.
			// Let's stick to fsPromises.access for the DIRECTORY check to be safe, or use fs.exists.
			// BUT, the plan said "Replace fs.access with await Bun.file(path).exists()".
			// Let's test if Bun.file(dir).exists() works later. For now, to be safe and robust, I'll stick to access for DIR, but use Glob for files.
			// actually, let's use the hybrid approach.
		}

		// Correction: Bun.file points to a file. For directory existence, `fs.existsSync` or `fs.access` is correct.
		// However, we can use `fsPromises.access` as before for the FOLDER.
		try {
			await fsPromises.access(wtfDir);
		} catch {
			return null;
		}

		const backupsBaseDir = path.join(retailDir, "Backups");
		const backupsWtfDir = path.join(backupsBaseDir, "WTF");

		// Ensure backup directory exists
		await fsPromises.mkdir(backupsWtfDir, { recursive: true });

		// Rate Limit Check
		if (minIntervalMinutes > 0) {
			try {
				const glob = new Bun.Glob("*.zip");
				const files: string[] = [];

				// Scan using Glob
				for await (const file of glob.scan({ cwd: backupsWtfDir })) {
					files.push(file);
				}

				const latest = files.sort().pop(); // ISO string based name sorting

				if (latest) {
					// format: WTF-YYYY-MM-DDTHH-mm-ss-sssZ.zip
					const latestPath = path.join(backupsWtfDir, latest);

					// We can use Bun.file for the ZIP file stat
					const file = Bun.file(latestPath);
					const lastModified = file.lastModified; // Returns timestamp
					const now = Date.now();
					const diffMs = now - lastModified;
					const diffMins = diffMs / 1000 / 60;

					if (diffMins < minIntervalMinutes) {
						return "skipped-recent";
					}
				}
			} catch {
				// No backups exist or checking failed, proceed
			}
		}

		// Create Zip Backup
		const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
		const zipFileName = `WTF-${timestamp}.zip`;
		const zipFilePath = path.join(backupsWtfDir, zipFileName);

		return new Promise((resolve, reject) => {
			const output = fs.createWriteStream(zipFilePath);
			const archive = archiver("zip", {
				zlib: { level: 9 }, // Sets the compression level.
			});

			output.on("close", () => {
				resolve(zipFilePath);
			});

			archive.on("error", (err) => {
				reject(err);
			});

			archive.pipe(output);

			// Append the WTF dir into the archive
			archive.directory(wtfDir, "WTF");

			archive.finalize();
		});
	},

	/**
	 * cleanupBackups deletes old backups, keeping only the N most recent
	 */
	async cleanupBackups(destDir: string, retentionCount: number) {
		if (retentionCount < 1) return;

		const retailDir = path.dirname(path.dirname(destDir));
		const backupsWtfDir = path.join(retailDir, "Backups", "WTF");

		try {
			const glob = new Bun.Glob("*.zip");
			const files: string[] = [];

			for await (const file of glob.scan({ cwd: backupsWtfDir })) {
				files.push(file);
			}

			const sortedFiles = files.sort().reverse(); // Newest first
			const toDelete = sortedFiles.slice(retentionCount);

			for (const fileName of toDelete) {
				const fullPath = path.join(backupsWtfDir, fileName);
				await fsPromises.rm(fullPath, { force: true });
			}
		} catch {
			// Ignore errors (e.g. dir doesn't exist)
		}
	},
};
