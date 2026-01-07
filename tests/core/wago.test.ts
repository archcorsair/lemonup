import { afterEach, describe, expect, it, mock, spyOn } from "bun:test";
import * as Wago from "@/core/wago";

describe("Wago API", () => {
	afterEach(() => {
		mock.restore();
	});

	describe("searchAddons", () => {
		it("should search for addons with API key", async () => {
			const mockResponse = {
				data: [
					{
						id: "clique",
						display_name: "Clique",
						summary: "Click-casting addon",
						thumbnail_image: null,
						categories: [],
						releases: {
							stable: {
								label: "1.0.0",
								logical_timestamp: 123,
								created_at: "2024-01-01",
							},
						},
						owner: "author",
						authors: ["author"],
						like_count: 100,
						download_count: 1000,
						website_url: "https://wago.io/clique",
					},
				],
			};

			const fetchSpy = spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);

			const result = await Wago.searchAddons("clique", "retail", "test-api-key");

			expect(fetchSpy).toHaveBeenCalledTimes(1);
			const call = fetchSpy.mock.calls[0];
			expect(call).toBeDefined();
			const [url, init] = call as [string, RequestInit | undefined];
			expect(url).toContain("_search");
			expect(url).toContain("query=clique");
			expect(url).toContain("game_version=retail");
			expect(init?.headers).toBeDefined();
			const headers = init?.headers as Record<string, string>;
			expect(headers.Authorization).toBe("Bearer test-api-key");
			expect(headers.Accept).toBe("application/json");

			expect(result).not.toBeNull();
			expect(result!.data).toHaveLength(1);
			expect(result!.data[0]!.id).toBe("clique");
		});

		it("should return null when no API key provided", async () => {
			const result = await Wago.searchAddons("clique", "retail");
			expect(result).toBeNull();
		});

		it("should return null on non-200 response", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response("", { status: 401 }),
			);

			const result = await Wago.searchAddons("clique", "retail", "bad-key");
			expect(result).toBeNull();
		});

		it("should return null on network error", async () => {
			spyOn(global, "fetch").mockRejectedValue(new Error("Network error"));

			const result = await Wago.searchAddons("clique", "retail", "test-api-key");
			expect(result).toBeNull();
		});

		it("should include stability parameter when provided", async () => {
			const mockResponse = { data: [] };
			const fetchSpy = spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);

			await Wago.searchAddons("clique", "retail", "test-api-key", "beta");

			const call = fetchSpy.mock.calls[0];
			expect(call).toBeDefined();
			const [url] = call as [string, RequestInit | undefined];
			expect(url).toContain("stability=beta");
		});

		it("should return null on invalid response format", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify({ invalid: "format" }), { status: 200 }),
			);

			const result = await Wago.searchAddons("clique", "retail", "test-api-key");
			expect(result).toBeNull();
		});
	});

	describe("getAddonDetails", () => {
		it("should fetch addon details by ID", async () => {
			const mockAddon = {
				id: "clique",
				display_name: "Clique",
				summary: "Click-casting addon",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: {
						label: "1.0.0",
						logical_timestamp: 123,
						created_at: "2024-01-01",
						download_link: "https://download.example.com",
					},
				},
				owner: "author",
				authors: ["author"],
				like_count: 100,
				download_count: 1000,
				website_url: "https://wago.io/clique",
			};

			spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockAddon), { status: 200 }),
			);

			const result = await Wago.getAddonDetails("clique", "test-api-key");
			expect(result.success).toBe(true);
			if (result.success) {
				expect(result.addon.display_name).toBe("Clique");
			}
		});

		it("should return not_found error for 404", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response("", { status: 404 }),
			);

			const result = await Wago.getAddonDetails("nonexistent", "test-api-key");
			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.error).toBe("not_found");
			}
		});

		it("should return no_api_key error when no API key provided", async () => {
			const result = await Wago.getAddonDetails("clique");
			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.error).toBe("no_api_key");
			}
		});

		it("should return network_error on non-404 error response", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response("", { status: 500 }),
			);

			const result = await Wago.getAddonDetails("clique", "test-api-key");
			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.error).toBe("network_error");
			}
		});

		it("should return network_error on fetch failure", async () => {
			spyOn(global, "fetch").mockRejectedValue(new Error("Network error"));

			const result = await Wago.getAddonDetails("clique", "test-api-key");
			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.error).toBe("network_error");
			}
		});

		it("should include game_version parameter when provided", async () => {
			const mockAddon = {
				id: "clique",
				display_name: "Clique",
				summary: "Click-casting addon",
				thumbnail_image: null,
				categories: [],
				releases: { stable: { label: "1.0.0", logical_timestamp: 123, created_at: "2024-01-01" } },
				owner: "author",
				authors: ["author"],
				like_count: 100,
				download_count: 1000,
				website_url: "https://wago.io/clique",
			};

			const fetchSpy = spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockAddon), { status: 200 }),
			);

			await Wago.getAddonDetails("clique", "test-api-key", "classic");

			const call = fetchSpy.mock.calls[0];
			expect(call).toBeDefined();
			const [url] = call as [string, RequestInit | undefined];
			expect(url).toContain("game_version=classic");
		});

		it("should return invalid_response error on malformed response", async () => {
			spyOn(global, "fetch").mockResolvedValue(
				new Response(JSON.stringify({ invalid: "data" }), { status: 200 }),
			);

			const result = await Wago.getAddonDetails("clique", "test-api-key");
			expect(result.success).toBe(false);
			if (!result.success) {
				expect(result.error).toBe("invalid_response");
			}
		});
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
