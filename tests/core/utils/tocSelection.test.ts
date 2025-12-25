import { describe, expect, test } from "bun:test";
import { selectTocFile } from "@/core/utils/tocSelection";

describe("selectTocFile", () => {
	describe("single TOC file", () => {
		test("should return the only TOC file with exact confidence", () => {
			const result = selectTocFile("WeakAuras", ["WeakAuras.toc"], "retail");
			expect(result.selected).toBe("WeakAuras.toc");
			expect(result.confidence).toBe("exact");
		});
	});

	describe("retail flavor", () => {
		test("should select -Retail suffix with exact confidence", () => {
			const result = selectTocFile(
				"WeakAuras",
				["WeakAuras.toc", "WeakAuras-Retail.toc", "WeakAuras-Classic.toc"],
				"retail",
			);
			expect(result.selected).toBe("WeakAuras-Retail.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should select _Mainline suffix with exact confidence", () => {
			const result = selectTocFile(
				"Details",
				["Details.toc", "Details_Mainline.toc", "Details_Classic.toc"],
				"retail",
			);
			expect(result.selected).toBe("Details_Mainline.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should fallback to base TOC when no retail-specific exists", () => {
			const result = selectTocFile(
				"SomeAddon",
				["SomeAddon.toc", "SomeAddon-Classic.toc"],
				"retail",
			);
			expect(result.selected).toBe("SomeAddon.toc");
			expect(result.confidence).toBe("fallback");
		});
	});

	describe("classic flavor", () => {
		test("should select -Classic suffix with exact confidence", () => {
			const result = selectTocFile(
				"WeakAuras",
				["WeakAuras.toc", "WeakAuras-Retail.toc", "WeakAuras-Classic.toc"],
				"classic",
			);
			expect(result.selected).toBe("WeakAuras-Classic.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should select _Classic suffix with exact confidence", () => {
			const result = selectTocFile(
				"Details",
				["Details.toc", "Details_Mainline.toc", "Details_Classic.toc"],
				"classic",
			);
			expect(result.selected).toBe("Details_Classic.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should select -Era suffix for classic era", () => {
			const result = selectTocFile(
				"ClassicLFG",
				["ClassicLFG.toc", "ClassicLFG-Era.toc"],
				"classic",
			);
			expect(result.selected).toBe("ClassicLFG-Era.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should fallback to base TOC when no classic-specific exists", () => {
			const result = selectTocFile(
				"SomeAddon",
				["SomeAddon.toc", "SomeAddon-Retail.toc"],
				"classic",
			);
			expect(result.selected).toBe("SomeAddon.toc");
			expect(result.confidence).toBe("fallback");
		});
	});

	describe("cata flavor", () => {
		test("should select -Cata suffix with exact confidence", () => {
			const result = selectTocFile(
				"WeakAuras",
				["WeakAuras.toc", "WeakAuras-Retail.toc", "WeakAuras-Cata.toc"],
				"cata",
			);
			expect(result.selected).toBe("WeakAuras-Cata.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should select _Cataclysm suffix with exact confidence", () => {
			const result = selectTocFile(
				"DBM",
				["DBM.toc", "DBM_Cataclysm.toc", "DBM-Retail.toc"],
				"cata",
			);
			expect(result.selected).toBe("DBM_Cataclysm.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should fallback to base TOC when no cata-specific exists", () => {
			const result = selectTocFile(
				"SomeAddon",
				["SomeAddon.toc", "SomeAddon-Retail.toc"],
				"cata",
			);
			expect(result.selected).toBe("SomeAddon.toc");
			expect(result.confidence).toBe("fallback");
		});
	});

	describe("ambiguous cases", () => {
		test("should return ambiguous when no base or flavor match", () => {
			const result = selectTocFile(
				"WeakAuras",
				["WA-Options.toc", "WA-Core.toc"],
				"retail",
			);
			expect(result.confidence).toBe("ambiguous");
			expect(result.selected).toBe("WA-Core.toc"); // First alphabetically
		});

		test("should pick first alphabetically when ambiguous", () => {
			const result = selectTocFile(
				"MyAddon",
				["Zebra.toc", "Alpha.toc", "Beta.toc"],
				"retail",
			);
			expect(result.selected).toBe("Alpha.toc");
			expect(result.confidence).toBe("ambiguous");
		});
	});

	describe("case insensitivity", () => {
		test("should match case-insensitively for flavor suffix", () => {
			const result = selectTocFile(
				"WeakAuras",
				["WeakAuras.toc", "weakauras-RETAIL.toc"],
				"retail",
			);
			expect(result.selected).toBe("weakauras-RETAIL.toc");
			expect(result.confidence).toBe("exact");
		});

		test("should match case-insensitively for base TOC", () => {
			const result = selectTocFile(
				"WeakAuras",
				["WEAKAURAS.TOC", "WeakAuras-Classic.toc"],
				"retail",
			);
			expect(result.selected).toBe("WEAKAURAS.TOC");
			expect(result.confidence).toBe("fallback");
		});
	});

	describe("error handling", () => {
		test("should throw when no TOC files provided", () => {
			expect(() => selectTocFile("WeakAuras", [], "retail")).toThrow(
				"No TOC files found",
			);
		});
	});

	describe("suffix priority", () => {
		test("should prefer -Retail over _Mainline for retail", () => {
			const result = selectTocFile(
				"TestAddon",
				["TestAddon-Retail.toc", "TestAddon_Mainline.toc"],
				"retail",
			);
			expect(result.selected).toBe("TestAddon-Retail.toc");
		});

		test("should prefer -Classic over _Classic for classic", () => {
			const result = selectTocFile(
				"TestAddon",
				["TestAddon-Classic.toc", "TestAddon_Classic.toc"],
				"classic",
			);
			expect(result.selected).toBe("TestAddon-Classic.toc");
		});
	});
});
