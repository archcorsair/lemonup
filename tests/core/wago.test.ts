import { afterEach, describe, expect, it, mock, spyOn } from "bun:test";
import * as Wago from "@/core/wago";

describe("Wago API", () => {
	afterEach(() => {
		mock.restore();
	});

	describe("getGameData", () => {
		it("should fetch and return game data", async () => {
			const mockResponse = {
				stability_values: ["stable", "beta", "alpha"],
				patches: { retail: ["11.0.2"], classic: ["1.15.0"] },
				toc_suffixes: { retail: ["-Mainline"], classic: ["-Classic"] },
				live_patches: { supported_retail_patches: "11.0.2" },
			};

			spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);

			const result = await Wago.getGameData();
			expect(result).not.toBeNull();
			expect(result?.stability_values).toContain("stable");
		});

		it("should return null on network error", async () => {
			spyOn(global, "fetch").mockRejectedValue(new Error("Network error"));

			const result = await Wago.getGameData();
			expect(result).toBeNull();
		});

		it("should return null on non-200 response", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response("", { status: 500 }),
			);

			const result = await Wago.getGameData();
			expect(result).toBeNull();
		});

		it("should return null on invalid response format", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify({ invalid: "data" }), { status: 200 }),
			);

			const result = await Wago.getGameData();
			expect(result).toBeNull();
		});
	});
});
