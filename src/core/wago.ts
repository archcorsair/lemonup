import { logger } from "@/core/logger";

// --- Types ---

export interface WagoGameData {
  stability_values: string[];
  patches: Record<string, string[]>;
  toc_suffixes: Record<string, string[]>;
  live_patches: Record<string, string>;
}

export interface WagoRelease {
  label: string;
  logical_timestamp: number;
  created_at: string;
  download_link?: string;
  link?: string;
  supported_patches?: string[];
  [key: string]: unknown; // Handle supported_<flavor>_patches dynamically
}

export interface WagoCategory {
  id: number;
  display_name: string;
  icon: string;
}

export interface WagoAddonSummary {
  id: string;
  display_name: string;
  summary: string;
  thumbnail_image: string | null;
  categories: WagoCategory[];
  releases: {
    stable?: WagoRelease;
    beta?: WagoRelease;
    alpha?: WagoRelease;
  };
  owner: string;
  authors: string[];
  like_count: number;
  download_count: number;
  website_url: string;
}

export interface WagoSearchResponse {
  data: WagoAddonSummary[];
}

export type WagoStability = "stable" | "beta" | "alpha";

export type GetAddonDetailsResult =
  | { success: true; addon: WagoAddonSummary }
  | {
      success: false;
      error: "not_found" | "network_error" | "invalid_response" | "no_api_key";
    };

export type WagoGameVersion =
  | "retail"
  | "classic"
  | "cata"
  | "wotlk"
  | "bc"
  | "mop";

// --- Constants ---

const API_BASE = "https://addons.wago.io";
const GAME_DATA_PATH = "/api/data/game";
const EXTERNAL_PATH = "/api/external";

// --- Helper Functions ---

function getHeaders(apiKey: string): Record<string, string> {
  return {
    Authorization: `Bearer ${apiKey}`,
    Accept: "application/json",
  };
}

function isValidSearchResponse(data: unknown): data is WagoSearchResponse {
  if (!data || typeof data !== "object") {
    return false;
  }
  const obj = data as Record<string, unknown>;
  return "data" in obj && Array.isArray(obj.data);
}

function isValidAddonResponse(data: unknown): data is WagoAddonSummary {
  if (!data || typeof data !== "object") {
    return false;
  }
  const obj = data as Record<string, unknown>;
  return (
    typeof obj.id === "string" &&
    typeof obj.display_name === "string" &&
    typeof obj.summary === "string" &&
    "releases" in obj &&
    typeof obj.releases === "object"
  );
}

// --- Utility Functions ---

/**
 * Extracts the download URL from an addon's release.
 * Handles API field drift: download_link vs link
 */
export function getDownloadUrl(
  addon: WagoAddonSummary,
  stability: WagoStability = "stable",
): string | null {
  const release = addon.releases[stability];
  if (!release) {
    return null;
  }
  // Handle field drift: download_link vs link
  return release.download_link ?? release.link ?? null;
}

/**
 * Extracts the version label from an addon's release.
 */
export function getVersion(
  addon: WagoAddonSummary,
  stability: WagoStability = "stable",
): string | null {
  const release = addon.releases[stability];
  return release?.label ?? null;
}

/**
 * Gets the best available stability channel for an addon.
 * Prefers stable > beta > alpha
 */
export function getBestAvailableStability(
  addon: WagoAddonSummary,
): WagoStability | null {
  if (addon.releases.stable) return "stable";
  if (addon.releases.beta) return "beta";
  if (addon.releases.alpha) return "alpha";
  return null;
}

/**
 * Extracts addon ID from a Wago URL.
 * Supports: https://addons.wago.io/addons/<id>
 *           https://wago.io/addons/<id>
 */
export function getAddonIdFromUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (!parsed.hostname.endsWith("wago.io")) {
      return null;
    }
    // Match /addons/<id> where id can contain alphanumerics, hyphens, and underscores
    const match = parsed.pathname.match(/\/addons\/([a-zA-Z0-9_-]+)/);
    return match?.[1] ?? null;
  } catch {
    return null;
  }
}

// --- API Functions ---

export async function getGameData(): Promise<WagoGameData | null> {
  const url = `${API_BASE}${GAME_DATA_PATH}`;
  logger.log("Wago", `Fetching game data from ${url}`);

  try {
    const response = await fetch(url, {
      headers: { Accept: "application/json" },
    });

    if (!response.ok) {
      logger.error("Wago", `Game data request failed: ${response.status}`);
      return null;
    }

    const data = (await response.json()) as unknown;

    // Basic structural validation
    if (
      !data ||
      typeof data !== "object" ||
      !("stability_values" in data) ||
      !Array.isArray((data as Record<string, unknown>).stability_values)
    ) {
      logger.error("Wago", "Invalid game data response format");
      return null;
    }

    return data as WagoGameData;
  } catch (error) {
    logger.error("Wago", "Failed to fetch game data", error);
    return null;
  }
}

export async function searchAddons(
  query: string,
  gameVersion: WagoGameVersion,
  apiKey?: string,
  stability?: WagoStability,
): Promise<WagoSearchResponse | null> {
  if (!apiKey) {
    logger.error("Wago", "API key required for search");
    return null;
  }

  const params = new URLSearchParams({
    query,
    game_version: gameVersion,
  });
  if (stability) {
    params.set("stability", stability);
  }

  const url = `${API_BASE}${EXTERNAL_PATH}/addons/_search?${params}`;
  logger.log("Wago", `Searching: ${url}`);

  try {
    const response = await fetch(url, {
      headers: getHeaders(apiKey),
    });

    if (!response.ok) {
      logger.error("Wago", `Search failed: ${response.status}`);
      return null;
    }

    const data = (await response.json()) as unknown;

    if (!isValidSearchResponse(data)) {
      logger.error("Wago", "Invalid search response format");
      return null;
    }

    return data;
  } catch (error) {
    logger.error("Wago", "Search request failed", error);
    return null;
  }
}

export async function getAddonDetails(
  addonId: string,
  apiKey?: string,
  gameVersion?: WagoGameVersion,
): Promise<GetAddonDetailsResult> {
  if (!apiKey) {
    return { success: false, error: "no_api_key" };
  }

  const params = gameVersion ? `?game_version=${gameVersion}` : "";
  const url = `${API_BASE}${EXTERNAL_PATH}/addons/${addonId}${params}`;
  logger.log("Wago", `Fetching addon details: ${url}`);

  try {
    const response = await fetch(url, {
      headers: getHeaders(apiKey),
    });

    if (!response.ok) {
      if (response.status === 404) {
        return { success: false, error: "not_found" };
      }
      logger.error("Wago", `Details request failed: ${response.status}`);
      return { success: false, error: "network_error" };
    }

    const data = (await response.json()) as unknown;

    if (!isValidAddonResponse(data)) {
      logger.error("Wago", "Invalid addon response format");
      return { success: false, error: "invalid_response" };
    }

    return { success: true, addon: data };
  } catch (error) {
    logger.error("Wago", "Failed to fetch addon details", error);
    return { success: false, error: "network_error" };
  }
}
