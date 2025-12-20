import { logger } from "./logger";

export interface WoWInterfaceAddonDetails {
	UID: string;
	UIName: string;
	UIVersion: string;
	UIDownload: string;
	UIAuthorName: string;
	UIFileName: string;
}

const API_BASE = "https://api.mmoui.com/v3/game/WOW/filedetails";

export function getAddonIdFromUrl(url: string): string | null {
	// Matches typical URLs:
	// https://wowinterface.com/downloads/info<id>-<name>.html
	// https://www.wowinterface.com/downloads/info<id>-<name>.html
	// https://www.wowinterface.com/downloads/info<id>.html
	const match = url.match(/info(\d+)/);
	return match?.[1] ? match[1] : null;
}

export async function getAddonDetails(
	id: string,
): Promise<WoWInterfaceAddonDetails | null> {
	const apiUrl = `${API_BASE}/${id}.json`;
	logger.log("WoWInterface", `Fetching details for ID ${id} from ${apiUrl}`);

	try {
		const response = await fetch(apiUrl);
		if (!response.ok) {
			logger.error(
				"WoWInterface",
				`API request failed: ${response.status} ${response.statusText}`,
			);
			return null;
		}

		const data = (await response.json()) as unknown;

		if (data && typeof data === "object" && "ERROR" in data) {
			const err = data as { ERROR: unknown };
			logger.error("WoWInterface", `API Error: ${String(err.ERROR)}`);
			return null;
		}

		if (!Array.isArray(data) || data.length === 0) {
			logger.error("WoWInterface", "Invalid API response format");
			return null;
		}

		return data[0] as WoWInterfaceAddonDetails;
	} catch (error) {
		logger.error("WoWInterface", "Failed to fetch addon details", error);
		return null;
	}
}
