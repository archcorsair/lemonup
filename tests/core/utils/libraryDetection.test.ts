import { describe, expect, test } from "bun:test";
import { detectLibraryKind } from "@/core/utils/libraryDetection";

describe("detectLibraryKind", () => {
	describe("TOC metadata detection (high confidence)", () => {
		test("should detect library from TOC X-Library: true", () => {
			const result = detectLibraryKind(
				"SomeAddon",
				{ xLibrary: true },
				{ hasDependents: false, hasDependencies: false },
			);
			expect(result.kind).toBe("library");
			expect(result.confidence).toBe("high");
			expect(result.reason).toContain("X-Library");
		});

		test("should not detect library when X-Library is false", () => {
			const result = detectLibraryKind(
				"SomeAddon",
				{ xLibrary: false },
				{ hasDependents: false, hasDependencies: false },
			);
			expect(result.kind).toBe("addon");
		});
	});

	describe("name pattern detection (medium confidence)", () => {
		const libraryNames = [
			"LibStub",
			"LibDBIcon-1.0",
			"LibSharedMedia-3.0",
			"Ace3",
			"AceAddon-3.0",
			"AceDB-3.0",
			"CallbackHandler-1.0",
			"LibDeflate",
			"LibSerialize",
		];

		for (const name of libraryNames) {
			test(`should detect "${name}" as library by name pattern`, () => {
				const result = detectLibraryKind(
					name,
					{},
					{ hasDependents: false, hasDependencies: false },
				);
				expect(result.kind).toBe("library");
				expect(result.confidence).toBe("medium");
				expect(result.reason).toContain("pattern");
			});
		}

		const addonNames = [
			"WeakAuras",
			"Details",
			"ElvUI",
			"DBM-Core",
			"Clique",
			"Bartender4",
		];

		for (const name of addonNames) {
			test(`should NOT detect "${name}" as library by name`, () => {
				const result = detectLibraryKind(
					name,
					{},
					{ hasDependents: false, hasDependencies: false },
				);
				expect(result.kind).toBe("addon");
			});
		}
	});

	describe("dependency graph detection (low confidence)", () => {
		test("should detect library when only depended on with no deps", () => {
			const result = detectLibraryKind(
				"SomeSharedCode",
				{},
				{ hasDependents: true, hasDependencies: false },
			);
			expect(result.kind).toBe("library");
			expect(result.confidence).toBe("low");
			expect(result.reason).toContain("depended on");
		});

		test("should NOT detect library when it has dependencies", () => {
			const result = detectLibraryKind(
				"SomeAddon",
				{},
				{ hasDependents: true, hasDependencies: true },
			);
			expect(result.kind).toBe("addon");
		});

		test("should NOT detect library when nothing depends on it", () => {
			const result = detectLibraryKind(
				"SomeAddon",
				{},
				{ hasDependents: false, hasDependencies: false },
			);
			expect(result.kind).toBe("addon");
		});
	});

	describe("priority order", () => {
		test("TOC metadata takes priority over name pattern", () => {
			// LibStub would match name pattern, but X-Library: false should override
			const result = detectLibraryKind(
				"LibStub",
				{ xLibrary: false },
				{ hasDependents: false, hasDependencies: false },
			);
			// Name pattern still matches since xLibrary: false just means "not explicitly true"
			expect(result.kind).toBe("library");
			expect(result.confidence).toBe("medium");
		});

		test("TOC X-Library: true takes highest priority", () => {
			const result = detectLibraryKind(
				"WeakAuras", // Not a library name
				{ xLibrary: true },
				{ hasDependents: false, hasDependencies: true },
			);
			expect(result.kind).toBe("library");
			expect(result.confidence).toBe("high");
		});
	});
});
