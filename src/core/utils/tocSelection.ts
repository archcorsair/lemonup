import type { GameFlavor } from "@/core/db";

export interface TocSelectionResult {
  selected: string;
  confidence: "exact" | "fallback" | "ambiguous";
}

/**
 * Flavor-specific TOC file suffixes.
 * Order matters - first match wins.
 */
const FLAVOR_SUFFIXES: Record<GameFlavor, string[]> = {
  retail: ["-Retail", "_Mainline", "-Mainline"],
  classic: ["-Classic", "_Classic", "-Vanilla", "_Vanilla", "-Era", "_Era"],
  cata: ["-Cata", "_Cata", "-Cataclysm", "_Cataclysm"],
};

/**
 * Selects the best matching TOC file for a given addon folder and target game flavor.
 *
 * Selection priority:
 * 1. Exact flavor-specific match (e.g., AddonName-Retail.toc for retail) -> "exact"
 * 2. Base TOC file (AddonName.toc) -> "fallback"
 * 3. If multiple matches or only non-matching TOCs exist -> "ambiguous" (first alphabetically)
 *
 * @param addonFolder - The addon folder name (e.g., "WeakAuras")
 * @param tocFiles - List of .toc file names found in the addon directory
 * @param targetFlavor - The game flavor to select for (retail, classic, cata)
 * @returns Selection result with the chosen file and confidence level
 */
export function selectTocFile(
  addonFolder: string,
  tocFiles: string[],
  targetFlavor: GameFlavor,
): TocSelectionResult {
  if (tocFiles.length === 0) {
    throw new Error(`No TOC files found for addon: ${addonFolder}`);
  }

  if (tocFiles.length === 1) {
    // Safe: we just checked length === 1
    return { selected: tocFiles[0] as string, confidence: "exact" };
  }

  const baseToc = `${addonFolder}.toc`;
  const suffixes = FLAVOR_SUFFIXES[targetFlavor];

  // Look for exact flavor-specific match
  for (const suffix of suffixes) {
    const flavorToc = `${addonFolder}${suffix}.toc`;
    const found = tocFiles.find(
      (f) => f.toLowerCase() === flavorToc.toLowerCase(),
    );
    if (found) {
      return { selected: found, confidence: "exact" };
    }
  }

  // Fallback to base TOC
  const baseFound = tocFiles.find(
    (f) => f.toLowerCase() === baseToc.toLowerCase(),
  );
  if (baseFound) {
    return { selected: baseFound, confidence: "fallback" };
  }

  // Ambiguous: no flavor match, no base match - pick first alphabetically
  const sorted = [...tocFiles].sort((a, b) =>
    a.toLowerCase().localeCompare(b.toLowerCase()),
  );
  // Safe: we checked tocFiles.length > 0 at the start
  return { selected: sorted[0] as string, confidence: "ambiguous" };
}
