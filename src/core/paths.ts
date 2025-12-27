import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { logger } from "./logger";

export function getDefaultWoWPath(): string {
  const platform = os.platform();
  const homedir = os.homedir();

  logger.logSync(
    "Paths",
    `Attempting auto-detection for platform: ${platform}`,
  );

  switch (platform) {
    case "win32": {
      const winDrives = ["C", "D", "E", "F", "G"];
      const winSuffix =
        ":\\Program Files (x86)\\World of Warcraft\\_retail_\\Interface\\AddOns";

      for (const drive of winDrives) {
        const p = drive + winSuffix;
        logger.logSync("Paths", `Checking Windows path: ${p}`);
        if (verifyWoWDirectory(p)) {
          logger.logSync("Paths", `Found WoW Retail at: ${p}`);
          return p;
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
    return false;
  }

  // addonPath is likely something like .../_retail_/Interface/AddOns
  // We look for WoW executables in the parent of Interface (the flavor folder)
  const separator = addonPath.includes("\\") ? "\\" : "/";
  const parts = addonPath.split(/[\\/]/);
  if (parts.length < 3) {
    logger.logSync("Paths", `Path too short for verification: ${addonPath}`);
    return false;
  }
  const flavorDir = parts.slice(0, -2).join(separator);

  const artifacts = [
    "Wow.exe",
    "Wow-64.exe",
    "World of Warcraft.app",
    ".build.info", // Common to all installs
    "Data",
  ];

  const found = artifacts.some((artifact) =>
    pathExists(flavorDir + separator + artifact),
  );

  if (!found) {
    logger.logSync(
      "Paths",
      `Verification failed for ${addonPath} (no artifacts found in ${flavorDir})`,
    );
  }

  return found;
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
