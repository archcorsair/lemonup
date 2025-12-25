import { logger } from "@/core/logger";

export interface TukUIAddon {
  id: number;
  slug: string;
  author: string;
  name: string;
  url: string;
  version: string;
  changelog_url: string;
  ticket_url: string;
  git_url: string;
  patch: string[];
  last_update: string;
  web_url: string;
  donate_url: string;
  small_desc: string;
  desc: string;
  screenshot_url: string;
  gallery_url: string[];
  logo_url: string;
  logo_square_url: string;
  directories: string[];
}

const API_URL = "https://api.tukui.org/v1/addons";

let cache: TukUIAddon[] | null = null;
let lastFetch = 0;
const CACHE_DURATION = 5 * 60 * 1000; // 5 minutes

export async function getAddons(force = false): Promise<TukUIAddon[]> {
  if (!force && cache && Date.now() - lastFetch < CACHE_DURATION) {
    return cache;
  }

  logger.log("TukUI", `Fetching addons from ${API_URL}`);
  try {
    const response = await fetch(API_URL);
    if (!response.ok) {
      logger.error(
        "TukUI",
        `API request failed: ${response.status} ${response.statusText}`,
      );
      return [];
    }

    const data = (await response.json()) as unknown;

    if (!Array.isArray(data)) {
      logger.error("TukUI", "Invalid API response format");
      return [];
    }

    cache = data as TukUIAddon[];
    lastFetch = Date.now();
    return cache;
  } catch (error) {
    logger.error("TukUI", "Failed to fetch addons", error);
    return [];
  }
}

export async function getAddonDetails(
  slugOrName: string,
): Promise<TukUIAddon | null> {
  const addons = await getAddons();
  const target = slugOrName.toLowerCase();
  return (
    addons.find(
      (a) =>
        a.slug.toLowerCase() === target ||
        a.name.toLowerCase() === target ||
        // Fallback for ElvUI since slug might be 'elvui' but name 'ElvUI'
        (target === "elvui" && a.slug === "elvui"),
    ) || null
  );
}
