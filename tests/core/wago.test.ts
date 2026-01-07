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

	describe("getDownloadUrl", () => {
		it("should extract download_link from stable release", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: {
						label: "1.0.0",
						logical_timestamp: 123,
						created_at: "2024-01-01",
						download_link: "https://download.example.com/stable",
					},
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getDownloadUrl(addon, "stable");
			expect(result).toBe("https://download.example.com/stable");
		});

		it("should fallback to link when download_link is missing", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: {
						label: "1.0.0",
						logical_timestamp: 123,
						created_at: "2024-01-01",
						link: "https://download.example.com/fallback",
					},
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getDownloadUrl(addon, "stable");
			expect(result).toBe("https://download.example.com/fallback");
		});

		it("should return null when no release exists for stability", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getDownloadUrl(addon, "stable");
			expect(result).toBeNull();
		});

		it("should use stable as default stability", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: {
						label: "1.0.0",
						logical_timestamp: 123,
						created_at: "2024-01-01",
						download_link: "https://download.example.com/stable",
					},
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getDownloadUrl(addon);
			expect(result).toBe("https://download.example.com/stable");
		});
	});

	describe("getVersion", () => {
		it("should extract version label from release", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: {
						label: "2.5.0",
						logical_timestamp: 123,
						created_at: "2024-01-01",
					},
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getVersion(addon, "stable");
			expect(result).toBe("2.5.0");
		});

		it("should return null when no release exists", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getVersion(addon, "stable");
			expect(result).toBeNull();
		});

		it("should use stable as default stability", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: {
						label: "3.0.0",
						logical_timestamp: 123,
						created_at: "2024-01-01",
					},
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getVersion(addon);
			expect(result).toBe("3.0.0");
		});
	});

	describe("getBestAvailableStability", () => {
		it("should prefer stable when available", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					stable: { label: "1.0.0", logical_timestamp: 123, created_at: "2024-01-01" },
					beta: { label: "1.1.0-beta", logical_timestamp: 124, created_at: "2024-01-02" },
					alpha: { label: "1.2.0-alpha", logical_timestamp: 125, created_at: "2024-01-03" },
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getBestAvailableStability(addon);
			expect(result).toBe("stable");
		});

		it("should fallback to beta when stable is not available", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					beta: { label: "1.1.0-beta", logical_timestamp: 124, created_at: "2024-01-02" },
					alpha: { label: "1.2.0-alpha", logical_timestamp: 125, created_at: "2024-01-03" },
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getBestAvailableStability(addon);
			expect(result).toBe("beta");
		});

		it("should fallback to alpha when stable and beta are not available", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {
					alpha: { label: "1.2.0-alpha", logical_timestamp: 125, created_at: "2024-01-03" },
				},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getBestAvailableStability(addon);
			expect(result).toBe("alpha");
		});

		it("should return null when no releases are available", () => {
			const addon: Wago.WagoAddonSummary = {
				id: "clique",
				display_name: "Clique",
				summary: "",
				thumbnail_image: null,
				categories: [],
				releases: {},
				owner: "author",
				authors: [],
				like_count: 0,
				download_count: 0,
				website_url: "",
			};

			const result = Wago.getBestAvailableStability(addon);
			expect(result).toBeNull();
		});
	});

	describe("getAddonIdFromUrl", () => {
		it("should extract addon ID from wago.io URL", () => {
			expect(Wago.getAddonIdFromUrl("https://addons.wago.io/addons/clique")).toBe("clique");
			expect(Wago.getAddonIdFromUrl("https://wago.io/addons/details-123")).toBe("details-123");
			expect(Wago.getAddonIdFromUrl("https://addons.wago.io/addons/my-addon/versions")).toBe("my-addon");
		});

		it("should handle addon IDs with underscores", () => {
			expect(Wago.getAddonIdFromUrl("https://addons.wago.io/addons/my_addon")).toBe("my_addon");
		});

		it("should return null for invalid URLs", () => {
			expect(Wago.getAddonIdFromUrl("https://example.com")).toBeNull();
			expect(Wago.getAddonIdFromUrl("not-a-url")).toBeNull();
		});

		it("should return null for non-wago domains", () => {
			expect(Wago.getAddonIdFromUrl("https://github.com/addons/clique")).toBeNull();
		});

		it("should return null for wago.io URLs without addon path", () => {
			expect(Wago.getAddonIdFromUrl("https://wago.io/")).toBeNull();
			expect(Wago.getAddonIdFromUrl("https://wago.io/other-path")).toBeNull();
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
