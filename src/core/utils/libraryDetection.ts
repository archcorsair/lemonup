import type { AddonKind } from "@/core/db";

export interface LibraryDetectionResult {
  kind: AddonKind;
  confidence: "high" | "medium" | "low";
  reason: string;
}

/**
 * Name patterns that indicate a library addon.
 * These are common WoW library naming conventions.
 */
const LIBRARY_NAME_PATTERNS = [
  /^Lib[A-Z]/, // LibStub, LibDBIcon, LibSharedMedia, etc.
  /^Ace[A-Z0-9]/, // Ace3, AceAddon, AceDB, etc.
  /-\d+\.\d+$/, // LibSharedMedia-3.0, CallbackHandler-1.0
  /^CallbackHandler/, // CallbackHandler-1.0
  /^LibStub$/, // LibStub
];

/**
 * Detects whether an addon should be classified as a library based on:
 * 1. Primary: TOC metadata (## X-Library: true) - highest confidence
 * 2. Secondary: Name patterns (Lib*, Ace*, *-1.0) - medium confidence
 * 3. Tertiary: Dependency graph (only depended on, no deps) - low confidence
 *
 * @param folderName - The addon folder name
 * @param tocMetadata - Parsed TOC metadata containing xLibrary flag
 * @param dependencyInfo - Information about the addon's dependency relationships
 * @returns Classification result with kind, confidence, and reason
 */
export function detectLibraryKind(
  folderName: string,
  tocMetadata: { xLibrary?: boolean },
  dependencyInfo: { hasDependents: boolean; hasDependencies: boolean },
): LibraryDetectionResult {
  // Primary: Explicit TOC metadata (highest confidence)
  if (tocMetadata.xLibrary === true) {
    return {
      kind: "library",
      confidence: "high",
      reason: "TOC X-Library: true",
    };
  }

  // Secondary: Name pattern matching (medium confidence)
  const matchesLibraryPattern = LIBRARY_NAME_PATTERNS.some((pattern) =>
    pattern.test(folderName),
  );
  if (matchesLibraryPattern) {
    return {
      kind: "library",
      confidence: "medium",
      reason: "Name matches library pattern",
    };
  }

  // Tertiary: Dependency graph heuristic (low confidence)
  // If something is depended upon but has no dependencies itself, likely a library
  if (dependencyInfo.hasDependents && !dependencyInfo.hasDependencies) {
    return {
      kind: "library",
      confidence: "low",
      reason: "Only depended on, has no dependencies",
    };
  }

  // Default: Regular addon
  return {
    kind: "addon",
    confidence: "high",
    reason: "Default classification",
  };
}
