import { describe, expect, test } from "bun:test";
import { getAddonIdFromUrl } from "@/core/wowinterface";

describe("WoWInterface Utils", () => {
	test("should extract ID from standard URL", () => {
		expect(
			getAddonIdFromUrl("https://wowinterface.com/downloads/info5108-Clique.html"),
		).toBe("5108");
	});

	test("should extract ID from www URL", () => {
		expect(
			getAddonIdFromUrl(
				"https://www.wowinterface.com/downloads/info5108-Clique.html",
			),
		).toBe("5108");
	});

	test("should extract ID from numeric only URL", () => {
		expect(
			getAddonIdFromUrl("https://www.wowinterface.com/downloads/info26105.html"),
		).toBe("26105");
	});

	test("should return null for invalid URL", () => {
		expect(getAddonIdFromUrl("https://google.com")).toBe(null);
		expect(
			getAddonIdFromUrl("https://wowinterface.com/downloads/nothing.html"),
		).toBe(null);
	});
});
