import {
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import archiver from "archiver";
import * as Downloader from "@/core/downloader";

const TMP_DIR = path.join(os.tmpdir(), "lemonup-tests-downloader");

describe("Downloader", () => {
	beforeEach(() => {
		fs.mkdirSync(TMP_DIR, { recursive: true });
	});

	afterEach(() => {
		if (fs.existsSync(TMP_DIR)) {
			fs.rmSync(TMP_DIR, { recursive: true, force: true });
		}
		mock.restore();
	});

	test("download should return true on success", async () => {
		const fetchSpy = spyOn(global, "fetch").mockResolvedValue(
			new Response("dummy content", { status: 200 }),
		);

		const dest = path.join(TMP_DIR, "downloaded.txt");
		const result = await Downloader.download("http://example.com/file", dest);

		expect(result).toBe(true);
		expect(fetchSpy).toHaveBeenCalledWith(
			"http://example.com/file",
			expect.any(Object),
		);
		expect(await Bun.file(dest).exists()).toBe(true);
		expect(await Bun.file(dest).text()).toBe("dummy content");
	});

	test("download should return false on various failures", async () => {
		// Network error (fetch throws)
		const fetchSpy = spyOn(global, "fetch").mockRejectedValue(
			new Error("Network"),
		);

		let result = await Downloader.download(
			"http://fail.com",
			path.join(TMP_DIR, "fail1"),
		);
		expect(result).toBe(false);

		// Non-200 status
		fetchSpy.mockResolvedValue(new Response("", { status: 404 }));

		result = await Downloader.download(
			"http://404.com",
			path.join(TMP_DIR, "fail2"),
		);
		expect(result).toBe(false);
	});

	test("unzip should extract files correctly", async () => {
		const zipPath = path.join(TMP_DIR, "test.zip");
		const extractDir = path.join(TMP_DIR, "extract");

		// Create a real zip file
		await new Promise((resolve, reject) => {
			const output = fs.createWriteStream(zipPath);
			const archive = archiver("zip");

			output.on("close", resolve);
			archive.on("error", reject);

			archive.pipe(output);
			archive.append("file-content", { name: "test-file.txt" });
			archive.append("nested-content", { name: "folder/nested.txt" });
			archive.finalize();
		});

		const result = await Downloader.unzip(zipPath, extractDir);

		expect(result).toBe(true);
		expect(
			await Bun.file(path.join(extractDir, "test-file.txt")).exists(),
		).toBe(true);
		expect(await Bun.file(path.join(extractDir, "test-file.txt")).text()).toBe(
			"file-content",
		);

		expect(
			await Bun.file(path.join(extractDir, "folder", "nested.txt")).exists(),
		).toBe(true);
		expect(
			await Bun.file(path.join(extractDir, "folder", "nested.txt")).text(),
		).toBe("nested-content");
	});

	test("unzip should throw on invalid zip", async () => {
		const badZipPath = path.join(TMP_DIR, "bad.zip");
		await Bun.write(badZipPath, "not a zip file");

		expect(Downloader.unzip(badZipPath, TMP_DIR)).rejects.toThrow(
			"Unzip execution failed",
		);
	});
});
