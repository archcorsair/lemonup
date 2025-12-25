import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export function getDefaultWoWPath(): string {
  const platform = os.platform();
  const homedir = os.homedir();

  switch (platform) {
    case "win32":
      return "C:\\Program Files (x86)\\World of Warcraft\\_retail_\\Interface\\AddOns";
    case "darwin":
      return "/Applications/World of Warcraft/_retail_/Interface/AddOns";
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
      ];

      for (const p of linuxPaths) {
        if (pathExists(p)) return p;
      }
      return "NOT_CONFIGURED";
    }
    default:
      return "NOT_CONFIGURED";
  }
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
