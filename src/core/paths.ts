import os from "node:os";
import path from "node:path";
import fs from "node:fs";

export function getDefaultWoWPath(): string {
	const platform = os.platform();
	const homedir = os.homedir();

	switch (platform) {
		case "win32":
			return "C:\\Program Files (x86)\\World of Warcraft\\_retail_\\Interface\\AddOns";
		case "darwin":
			return "/Applications/World of Warcraft/_retail_/Interface/AddOns";
		case "linux":
			// Check for common Linux locations (Wine, Lutris, Steam)
			// Priority: Lutris -> Wine -> Steam -> Fallback
			
			// Lutris default
			const lutrisPath = path.join(homedir, "Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns");
			// Wine default
			const winePath = path.join(homedir, ".wine/drive_c/Program Files (x86)/World of Warcraft/_retail_/Interface/AddOns");
			
			// Simple check if paths exist could be added here, but for now we return a sensible default or list.
			// Since we need to return ONE string, we return the most likely "standard" wine prefix.
			return winePath; 
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
