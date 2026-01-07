import { z } from "zod";
import { logger } from "@/core/logger";

// --- Zod Schemas ---

const WagoReleaseSchema = z
  .object({
    label: z.string(),
    logical_timestamp: z.number().optional(),
    created_at: z.string().optional(),
    download_link: z.string().optional(),
    link: z.string().optional(),
    supported_patches: z.array(z.string()).optional(),
  })
  .loose();

const WagoReleasesSchema = z.object({
  stable: WagoReleaseSchema.optional(),
  beta: WagoReleaseSchema.optional(),
  alpha: WagoReleaseSchema.optional(),
});

const WagoCategorySchema = z.object({
  id: z.number(),
  display_name: z.string(),
  icon: z.string(),
});

const WagoAddonSummarySchema = z
  .object({
    id: z.string(),
    display_name: z.string(),
    summary: z.string(),
    thumbnail_image: z.string().nullable(),
    categories: z.array(WagoCategorySchema).optional(),
    releases: WagoReleasesSchema.optional(),
    recent_release: WagoReleasesSchema.optional(),
    owner: z.string().optional(),
    authors: z.array(z.string()).optional(),
    like_count: z.number().optional(),
    download_count: z.number().optional(),
    website_url: z.string().optional(),
  })
  .loose();

const WagoSearchResponseSchema = z.object({
  data: z.array(WagoAddonSummarySchema),
});

const WagoGameDataSchema = z.object({
  stability_values: z.array(z.string()),
  patches: z.record(z.string(), z.array(z.string())),
  toc_suffixes: z.record(z.string(), z.array(z.string())),
  live_patches: z.record(z.string(), z.string()),
});

// --- Types (derived from Zod schemas) ---

export type WagoRelease = z.infer<typeof WagoReleaseSchema>;
export type WagoCategory = z.infer<typeof WagoCategorySchema>;
export type WagoGameData = z.infer<typeof WagoGameDataSchema>;
export type WagoSearchResponse = z.infer<typeof WagoSearchResponseSchema>;

export interface WagoAddonSummary {
  id: string;
  display_name: string;
  summary: string;
  thumbnail_image: string | null;
  categories?: WagoCategory[];
  releases: {
    stable?: WagoRelease;
    beta?: WagoRelease;
    alpha?: WagoRelease;
  };
  owner?: string;
  authors?: string[];
  like_count?: number;
  download_count?: number;
  website_url?: string;
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

    const json = await response.json();
    const result = WagoGameDataSchema.safeParse(json);

    if (!result.success) {
      logger.error("Wago", "Invalid game data response format");
      return null;
    }

    return result.data;
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

    const json = await response.json();
    const result = WagoSearchResponseSchema.safeParse(json);

    if (!result.success) {
      logger.error("Wago", "Invalid search response format");
      return null;
    }

    return result.data;
  } catch (error) {
    logger.error("Wago", "Search request failed", error);
    return null;
  }
}

// Schema for wrapped response format
const WagoWrappedResponseSchema = z.object({
  data: WagoAddonSummarySchema,
});

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

    const json = await response.json();

    // Try wrapped format first, then direct format
    const wrappedResult = WagoWrappedResponseSchema.safeParse(json);
    const directResult = WagoAddonSummarySchema.safeParse(json);

    const parsed = wrappedResult.success
      ? wrappedResult.data.data
      : directResult.success
        ? directResult.data
        : null;

    if (!parsed) {
      logger.error("Wago", "Invalid addon response format");
      return { success: false, error: "invalid_response" };
    }

    // Normalize: ensure releases exists (from recent_release if needed)
    const addon: WagoAddonSummary = {
      ...parsed,
      releases: parsed.releases ??
        parsed.recent_release ?? { stable: undefined },
    };

    return { success: true, addon };
  } catch (error) {
    logger.error("Wago", "Failed to fetch addon details", error);
    return { success: false, error: "network_error" };
  }
}
