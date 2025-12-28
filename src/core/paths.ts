import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { logger } from "./logger";

/**
 * Get available drive letters on Windows using PowerShell.
 * Falls back to hardcoded C-G on error.
 */
async function getAvailableWindowsDrives(): Promise<string[]> {
  if (os.platform() !== "win32") return [];

  try {
    const proc = Bun.spawn(
      ["powershell", "-Command", "(Get-PSDrive -PSProvider FileSystem).Root"],
      {
        stdout: "pipe",
        stderr: "pipe",
      },
    );

    const output = await new Response(proc.stdout).text();
    const exitCode = await proc.exited;

    if (exitCode !== 0) {
      return ["C", "D", "E", "F", "G"];
    }

    // Output format: "C:\nD:\nE:\n" -> ["C", "D", "E"]
    const drives = output
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => /^[A-Z]:\\$/.test(line))
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

  logger.logSync("Paths", `Auto-detection started (platform: ${platform})`);

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
          if (verifyWoWDirectory(p)) {
            logger.logSync("Paths", `Auto-detection found: ${p}`);
            return p;
          }
        }
      }
      break;
    }
    case "darwin": {
      const p = "/Applications/World of Warcraft/_retail_/Interface/AddOns";
      if (verifyWoWDirectory(p)) {
        logger.logSync("Paths", `Auto-detection found: ${p}`);
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
        if (verifyWoWDirectory(p)) {
          logger.logSync("Paths", `Auto-detection found: ${p}`);
          return p;
        }
      }
      break;
    }
  }

  logger.logSync(
    "Paths",
    "Auto-detection completed: no WoW installation found",
  );
  return "NOT_CONFIGURED";
}

export function verifyWoWDirectory(addonPath: string): boolean {
  if (!pathExists(addonPath)) {
    return false;
  }

  // Explicit retail-only enforcement
  const pathLower = addonPath.toLowerCase();
  if (pathLower.includes("_classic_") || pathLower.includes("_classic_era_")) {
    return false;
  }

  if (!pathLower.includes("_retail_")) {
    return false;
  }

  const separator = addonPath.includes("\\") ? "\\" : "/";
  const parts = addonPath.split(/[\\/]/);

  if (parts.length < 3) {
    return false;
  }

  // Verify path structure contains Interface/AddOns
  if (!parts.includes("Interface") || !parts.includes("AddOns")) {
    return false;
  }

  // wowRoot = World of Warcraft/ (contains Data, .build.info)
  // flavorDir = World of Warcraft/_retail_/ (contains Wow.exe)
  // Path structure: .../World of Warcraft/_retail_/Interface/AddOns
  // slice(0, -3) removes: _retail_, Interface, AddOns → wowRoot
  // slice(0, -2) removes: Interface, AddOns → flavorDir
  const wowRoot = parts.slice(0, -3).join(separator);
  const flavorDir = parts.slice(0, -2).join(separator);

  // Artifacts at WoW root level
  const rootArtifacts = ["Data", ".build.info"];
  // Artifacts inside the flavor directory (_retail_)
  const flavorArtifacts = ["Wow.exe", "Wow-64.exe", "World of Warcraft.app"];

  const foundArtifacts: string[] = [];

  for (const artifact of rootArtifacts) {
    const artifactPath = wowRoot + separator + artifact;
    if (pathExists(artifactPath)) {
      foundArtifacts.push(artifact);
    }
  }

  for (const artifact of flavorArtifacts) {
    const artifactPath = flavorDir + separator + artifact;
    if (pathExists(artifactPath)) {
      foundArtifacts.push(artifact);
    }
  }

  // Require at least 2 artifacts for confidence
  if (foundArtifacts.length < 2) {
    return false;
  }

  logger.logSync(
    "Paths",
    `Verified WoW path: ${addonPath} (${foundArtifacts.length} artifacts: ${foundArtifacts.join(", ")})`,
  );
  return true;
}

/**
 * Quick check of common WoW paths relative to a root directory.
 * Avoids deep scan by checking likely locations first.
 */
export async function quickCheckCommonPaths(
  rootPath: string,
): Promise<string | null> {
  const platform = os.platform();
  const candidates: string[] = [];

  if (platform === "win32") {
    candidates.push(
      "Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
      "Program Files/World of Warcraft/_retail_/Interface/AddOns",
      "Games/World of Warcraft/_retail_/Interface/AddOns",
      "World of Warcraft/_retail_/Interface/AddOns",
    );
  } else if (platform === "linux") {
    candidates.push(
      "Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
      ".wine/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns",
      ".local/share/Steam/steamapps/common/World of Warcraft/_retail_/Interface/AddOns",
    );
  } else if (platform === "darwin") {
    candidates.push("Applications/World of Warcraft/_retail_/Interface/AddOns");
  }

  for (const candidate of candidates) {
    const fullPath = path.join(rootPath, candidate);
    if (verifyWoWDirectory(fullPath)) {
      return fullPath;
    }
  }

  return null;
}

export async function searchForWoW(
  root: string,
  signal?: AbortSignal,
  onProgress?: (dirsScanned: number, currentPath: string) => void,
): Promise<string | null> {
  // Check if root directory is readable
  try {
    fs.accessSync(root, fs.constants.R_OK);
  } catch {
    throw new Error(`Cannot read directory: ${root} (permission denied)`);
  }

  logger.logSync("Scan", `Deep scan started from: ${root}`);
  const startTime = Date.now();

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
  const YIELD_INTERVAL = 50; // Reduced context switches (was 10)

  while (queue.length > 0) {
    if (signal?.aborted) {
      logger.logSync(
        "Scan",
        `Deep scan cancelled after ${dirsScanned} directories (${Date.now() - startTime}ms)`,
      );
      return null;
    }

    const currentDir = queue.shift();
    if (!currentDir) continue;

    dirsScanned++;
    onProgress?.(dirsScanned, currentDir);

    // Yield to event loop periodically
    if (dirsScanned % YIELD_INTERVAL === 0) {
      await new Promise((resolve) => setImmediate(resolve));
      // Check abort again after yielding
      if (signal?.aborted) {
        logger.logSync(
          "Scan",
          `Deep scan cancelled after ${dirsScanned} directories (${Date.now() - startTime}ms)`,
        );
        return null;
      }
    }

    try {
      const currentDepth = currentDir.split(path.sep).length - rootDepth;
      if (currentDepth > maxDepth) continue;

      const entries = fs.readdirSync(currentDir, { withFileTypes: true });

      // Check for _retail_ folder first in current entries
      const retailEntry = entries.find(
        (e) => e.name === "_retail_" && e.isDirectory(),
      );
      if (retailEntry) {
        const retailPath = path.join(currentDir, retailEntry.name);
        const addonsPath = path.join(retailPath, "Interface", "AddOns");
        if (verifyWoWDirectory(addonsPath)) {
          logger.logSync(
            "Scan",
            `Deep scan found WoW after ${dirsScanned} directories (${Date.now() - startTime}ms): ${addonsPath}`,
          );
          return addonsPath;
        }
      }

      // Add subdirectories to queue
      for (const entry of entries) {
        if (!entry.isDirectory()) continue;
        if (IGNORED_DIRS.has(entry.name)) continue;

        const fullPath = path.join(currentDir, entry.name);

        try {
          // Avoid symlink loops
          const stats = fs.lstatSync(fullPath);
          if (stats.isSymbolicLink()) continue;

          queue.push(fullPath);
        } catch {
          // Skip folders we can't access
        }
      }
    } catch {
      // Skip folders we can't read
    }
  }

  logger.logSync(
    "Scan",
    `Deep scan completed: not found after ${dirsScanned} directories (${Date.now() - startTime}ms)`,
  );
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
