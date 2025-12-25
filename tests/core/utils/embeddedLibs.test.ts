import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { detectEmbeddedLibs } from "@/core/utils/embeddedLibs";

describe("detectEmbeddedLibs", () => {
	let tempDir: string;

	beforeEach(async () => {
		tempDir = path.join(os.tmpdir(), `embeddedLibs-test-${crypto.randomUUID()}`);
		await fs.mkdir(tempDir, { recursive: true });
	});

	afterEach(async () => {
		await fs.rm(tempDir, { recursive: true, force: true });
	});

	test("should return empty array when no Libs directory exists", async () => {
		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual([]);
	});

	test("should return empty array when Libs directory is empty", async () => {
		await fs.mkdir(path.join(tempDir, "Libs"));
		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual([]);
	});

	test("should detect library with matching .toc file", async () => {
		const libPath = path.join(tempDir, "Libs", "LibStub");
		await fs.mkdir(libPath, { recursive: true });
		await fs.writeFile(path.join(libPath, "LibStub.toc"), "## Title: LibStub");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual(["LibStub"]);
	});

	test("should detect library with non-matching .toc file", async () => {
		const libPath = path.join(tempDir, "Libs", "SomeLib");
		await fs.mkdir(libPath, { recursive: true });
		await fs.writeFile(path.join(libPath, "DifferentName.toc"), "## Title: Some Library");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual(["SomeLib"]);
	});

	test("should detect multiple embedded libraries", async () => {
		// Library 1
		const lib1Path = path.join(tempDir, "Libs", "LibStub");
		await fs.mkdir(lib1Path, { recursive: true });
		await fs.writeFile(path.join(lib1Path, "LibStub.toc"), "## Title: LibStub");

		// Library 2
		const lib2Path = path.join(tempDir, "Libs", "LibDBIcon-1.0");
		await fs.mkdir(lib2Path, { recursive: true });
		await fs.writeFile(path.join(lib2Path, "LibDBIcon-1.0.toc"), "## Title: LibDBIcon");

		// Library 3
		const lib3Path = path.join(tempDir, "Libs", "CallbackHandler-1.0");
		await fs.mkdir(lib3Path, { recursive: true });
		await fs.writeFile(path.join(lib3Path, "CallbackHandler-1.0.toc"), "## Title: CallbackHandler");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result.sort()).toEqual(["CallbackHandler-1.0", "LibDBIcon-1.0", "LibStub"]);
	});

	test("should ignore directories without .toc files", async () => {
		// Directory with .toc
		const libPath = path.join(tempDir, "Libs", "LibStub");
		await fs.mkdir(libPath, { recursive: true });
		await fs.writeFile(path.join(libPath, "LibStub.toc"), "## Title: LibStub");

		// Directory without .toc
		const noTocPath = path.join(tempDir, "Libs", "SomeFolder");
		await fs.mkdir(noTocPath, { recursive: true });
		await fs.writeFile(path.join(noTocPath, "readme.txt"), "Just a readme");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual(["LibStub"]);
	});

	test("should ignore files in Libs directory (only scan subdirectories)", async () => {
		await fs.mkdir(path.join(tempDir, "Libs"));
		await fs.writeFile(path.join(tempDir, "Libs", "somefile.toc"), "## Title: Something");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual([]);
	});

	test("should check lowercase 'libs' directory", async () => {
		const libPath = path.join(tempDir, "libs", "LibDeflate");
		await fs.mkdir(libPath, { recursive: true });
		await fs.writeFile(path.join(libPath, "LibDeflate.toc"), "## Title: LibDeflate");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual(["LibDeflate"]);
	});

	test("should check 'Lib' directory variant", async () => {
		const libPath = path.join(tempDir, "Lib", "LibSerialize");
		await fs.mkdir(libPath, { recursive: true });
		await fs.writeFile(path.join(libPath, "LibSerialize.toc"), "## Title: LibSerialize");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual(["LibSerialize"]);
	});

	test("should check 'Libraries' directory variant", async () => {
		const libPath = path.join(tempDir, "Libraries", "AceAddon-3.0");
		await fs.mkdir(libPath, { recursive: true });
		await fs.writeFile(path.join(libPath, "AceAddon-3.0.toc"), "## Title: AceAddon");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result).toEqual(["AceAddon-3.0"]);
	});

	test("should combine libraries from multiple lib directory variants", async () => {
		// Library in Libs/
		const libs1Path = path.join(tempDir, "Libs", "LibStub");
		await fs.mkdir(libs1Path, { recursive: true });
		await fs.writeFile(path.join(libs1Path, "LibStub.toc"), "## Title: LibStub");

		// Library in lib/
		const libs2Path = path.join(tempDir, "lib", "LibDeflate");
		await fs.mkdir(libs2Path, { recursive: true });
		await fs.writeFile(path.join(libs2Path, "LibDeflate.toc"), "## Title: LibDeflate");

		const result = await detectEmbeddedLibs(tempDir);
		expect(result.sort()).toEqual(["LibDeflate", "LibStub"]);
	});
});
