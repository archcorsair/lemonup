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
    const zip = new AdmZip(zipPath);
    const zipEntries = zip.getEntries();

    const resolvedDestDir = path.resolve(destDir);
    const destDirWithSep = resolvedDestDir.endsWith(path.sep)
      ? resolvedDestDir
      : resolvedDestDir + path.sep;

    for (const entry of zipEntries) {
      const entryName = entry.entryName;

      // Robust path traversal check: normalize and resolve against the destination
      const entryPath = path.resolve(
        resolvedDestDir,
        path.normalize(entryName),
      );

      if (!entryPath.startsWith(destDirWithSep)) {
        logger.error("Downloader", `Skipping unsafe entry: ${entryName}`);
        continue;
      }

      if (entry.isDirectory) {
        await fsp.mkdir(entryPath, { recursive: true });
      } else {
        const parentDir = path.dirname(entryPath);
        await fsp.mkdir(parentDir, { recursive: true });

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
