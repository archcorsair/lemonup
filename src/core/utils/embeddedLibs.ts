import fs from "node:fs/promises";
import path from "node:path";

/**
 * Common directory names where WoW addons embed their libraries.
 */
const EMBEDDED_LIB_DIRS = ["Libs", "libs", "Lib", "lib", "Libraries"];

/**
 * Scans an addon folder for embedded libraries.
 * Embedded libraries are typically found in a Libs/ subdirectory
 * and contain their own .toc files.
 *
 * @param addonPath - Full path to the addon folder
 * @returns List of embedded library names (folder names, not paths)
 */
export async function detectEmbeddedLibs(addonPath: string): Promise<string[]> {
  const embedded: string[] = [];

  for (const libDir of EMBEDDED_LIB_DIRS) {
    const libsPath = path.join(addonPath, libDir);

    try {
      await fs.access(libsPath);
    } catch {
      // Directory doesn't exist, try next
      continue;
    }

    const entries = await fs.readdir(libsPath, { withFileTypes: true });

    for (const entry of entries) {
      if (!entry.isDirectory()) continue;

      // Check if this subdirectory has a .toc file (making it a valid library)
      const tocPath = path.join(libsPath, entry.name, `${entry.name}.toc`);
      try {
        await fs.access(tocPath);
        embedded.push(entry.name);
      } catch {
        // No matching .toc file, might still be a library with different naming
        // Try to find any .toc file in the directory
        const subEntries = await fs.readdir(path.join(libsPath, entry.name));
        const hasToc = subEntries.some((f) => f.endsWith(".toc"));
        if (hasToc) {
          embedded.push(entry.name);
        }
      }
    }
  }

  // Dedupe to avoid duplicate libs when multiple lib directory variants
  // (e.g. Libs vs libs) point to the same physical folders on case-insensitive filesystems.
  return Array.from(new Set(embedded));
}
