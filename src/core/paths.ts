import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { logger } from "./logger";

/**
 * Get available drive letters on Windows using wmic.
 * Falls back to hardcoded C-G on error.
 */
async function getAvailableWindowsDrives(): Promise<string[]> {
  if (os.platform() !== "win32") return [];

  try {
    const proc = Bun.spawn(["wmic", "logicaldisk", "get", "name"], {
      stdout: "pipe",
      stderr: "pipe",
    });

    const output = await new Response(proc.stdout).text();
    const exitCode = await proc.exited;

    if (exitCode !== 0) {
      return ["C", "D", "E", "F", "G"];
    }

    const drives = output
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => /^[A-Z]:$/.test(line))
      .map((line) => line[0])
      .filter((d): d is string => d !== undefined);

    return drives.length > 0 ? drives : ["C", "D", "E", "F", "G"];
  } catch {
    return ["C", "D", "E", "F", "G"];
  }
}

export async function getDefaultWoWPath(): Promise<string> {
  const platform = os.platform();
  const homedir = os.homedir();

  logger.logSync(
    "Paths",
    `Attempting auto-detection for platform: ${platform}`,
  );

  switch (platform) {
    case "win32": {
      const winDrives = await getAvailableWindowsDrives();
      const winPathTemplates = [
        ":\\Program Files (x86)\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ":\\Program Files\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ":\\Games\\World of Warcraft\\_retail_\\Interface\\AddOns",
        ":\\World of Warcraft\\_retail_\\Interface\\AddOns",
      ];

      for (const drive of winDrives) {
        for (const template of winPathTemplates) {
          const p = drive + template;
          logger.logSync("Paths", `Checking Windows path: ${p}`);
          if (verifyWoWDirectory(p)) {
            logger.logSync("Paths", `Found WoW Retail at: ${p}`);
            return p;
          }
        }
      }
      break;
    }
    case "darwin": {
      const p = "/Applications/World of Warcraft/_retail_/Interface/AddOns";
      logger.logSync("Paths", `Checking macOS path: ${p}`);
      if (verifyWoWDirectory(p)) {
        logger.logSync("Paths", `Found WoW Retail at: ${p}`);
        return p;
      }
      break;
    }
    case "linux": {
      const linuxPaths = [
        path.join(
          homedir,
          "Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
        ),
        path.join(
          homedir,
          ".wine/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
        ),
        path.join(
          homedir,
          ".var/app/com.usebottles.bottles/data/bottles/bottles/World-of-Warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
        ),
        path.join(
          homedir,
          ".local/share/Steam/steamapps/common/World of Warcraft/_retail_/Interface/AddOns",
        ),
        path.join(
          homedir,
          ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/World of Warcraft/_retail_/Interface/AddOns",
        ),
      ];

      for (const p of linuxPaths) {
        logger.logSync("Paths", `Checking Linux path: ${p}`);
        if (verifyWoWDirectory(p)) {
          logger.logSync("Paths", `Found WoW Retail at: ${p}`);
          return p;
        }
      }
      break;
    }
  }

  logger.logSync("Paths", "Auto-detection failed to find a valid Retail path");
  return "NOT_CONFIGURED";
}

export function verifyWoWDirectory(addonPath: string): boolean {
  if (!pathExists(addonPath)) {
    logger.logSync("Paths", `Path does not exist: ${addonPath}`);
    return false;
  }

  // Explicit retail-only enforcement
  const pathLower = addonPath.toLowerCase();
  if (pathLower.includes("_classic_") || pathLower.includes("_classic_era_")) {
    logger.logSync(
      "Paths",
      `Rejected non-retail path (Classic/Era not supported): ${addonPath}`,
    );
    return false;
  }

  if (!pathLower.includes("_retail_")) {
    logger.logSync(
      "Paths",
      `Rejected path missing _retail_ flavor: ${addonPath}`,
    );
    return false;
  }

  const separator = addonPath.includes("\\") ? "\\" : "/";
  const parts = addonPath.split(/[\\/]/);

  if (parts.length < 3) {
    logger.logSync("Paths", `Path too short for verification: ${addonPath}`);
    return false;
  }

  // Verify path structure contains Interface/AddOns
  if (!parts.includes("Interface") || !parts.includes("AddOns")) {
    logger.logSync("Paths", `Missing Interface/AddOns structure: ${addonPath}`);
    return false;
  }

  logger.logSync(
    "Paths",
    `Constructing flavor directory from path parts: ${parts.join(" > ")}`,
  );

  const flavorDir = parts.slice(0, -3).join(separator);

  logger.logSync(
    "Paths",
    `Checking for artifacts in flavor directory: ${flavorDir}`,
  );

  const artifacts = [
    "Wow.exe",
    "Wow-64.exe",
    "World of Warcraft.app",
    ".build.info",
    "Data",
  ];

  const foundArtifacts: string[] = [];
  for (const artifact of artifacts) {
    const artifactPath = flavorDir + separator + artifact;
    const exists = pathExists(artifactPath);
    if (exists) {
      foundArtifacts.push(artifact);
      logger.logSync("Paths", `  ✓ Found artifact: ${artifact}`);
    } else {
      logger.logSync("Paths", `  ✗ Artifact not found: ${artifact}`);
    }
  }

  // Require at least 2 artifacts for confidence
  if (foundArtifacts.length < 2) {
    logger.logSync(
      "Paths",
      `✓ Verification failed for ${addonPath}: only found ${foundArtifacts.length} artifact(s): ${foundArtifacts.join(", ")}`,
    );
    return false;
  }

  logger.logSync(
    "Paths",
    `Verified ${addonPath} with ${foundArtifacts.length} artifacts: ${foundArtifacts.join(", ")}`,
  );
  return true;
}

export async function searchForWoW(
  root: string,
  signal?: AbortSignal,
  onProgress?: (dirsScanned: number, currentPath: string) => void,
): Promise<string | null> {
  const IGNORED_DIRS = new Set([
    // Development
    "node_modules",
    ".git",
    // System (macOS)
    "Library",
    "System",
    "Applications",
    "private",
    // System (Linux)
    "proc",
    "sys",
    "dev",
    "boot",
    "snap",
    "flatpak",
    "lost+found",
    "run",
    "tmp",
    // System (Windows)
    "Windows",
    "$Recycle.Bin",
    "System Volume Information",
    "ProgramData",
    "AppData",
  ]);

  const queue: string[] = [root];
  const maxDepth = 10;
  const rootDepth = root.split(path.sep).length;
  let dirsScanned = 0;
  const YIELD_INTERVAL = 10; // Yield every 10 directories

  while (queue.length > 0) {
    if (signal?.aborted) return null;

    const currentDir = queue.shift();
    if (!currentDir) continue;

    dirsScanned++;
    onProgress?.(dirsScanned, currentDir);

    // Yield to event loop periodically
    if (dirsScanned % YIELD_INTERVAL === 0) {
      await new Promise((resolve) => setImmediate(resolve));
      // Check abort again after yielding
      if (signal?.aborted) return null;
    }

    try {
      const currentDepth = currentDir.split(path.sep).length - rootDepth;
      if (currentDepth > maxDepth) continue;

      const entries = fs.readdirSync(currentDir);

      // Check for _retail_ folder first in current entries
      if (entries.includes("_retail_")) {
        const retailPath = path.join(currentDir, "_retail_");
        const addonsPath = path.join(retailPath, "Interface", "AddOns");
        if (verifyWoWDirectory(addonsPath)) {
          return addonsPath;
        }
      }

      // Add subdirectories to queue
      for (const entry of entries) {
        if (IGNORED_DIRS.has(entry)) continue;

        const fullPath = path.join(currentDir, entry);
        try {
          const stats = fs.statSync(fullPath);
          if (stats.isDirectory()) {
            queue.push(fullPath);
          }
        } catch {
          // Skip folders we can't access
        }
      }
    } catch {
      // Skip folders we can't read
    }
  }

  return null;
}

export function isPathConfigured(pathStr: string): boolean {
  return pathStr !== "NOT_CONFIGURED" && pathStr.length > 0;
}

export function pathExists(p: string): boolean {
  try {
    return fs.existsSync(p);
  } catch {
    return false;
  }
}
