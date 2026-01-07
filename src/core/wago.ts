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
