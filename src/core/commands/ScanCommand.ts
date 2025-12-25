import fs from "node:fs/promises";
import path from "node:path";
import type { ConfigManager } from "@/core/config";
import type { AddonRecord, DatabaseManager, GameFlavor } from "@/core/db";
import * as GitClient from "@/core/git";
import { logger } from "@/core/logger";
import { detectEmbeddedLibs } from "@/core/utils/embeddedLibs";
import { detectLibraryKind } from "@/core/utils/libraryDetection";
import { selectTocFile } from "@/core/utils/tocSelection";
import type { Command, CommandContext } from "./types";

/**
 * Parsed TOC metadata from a WoW addon .toc file.
 */
export interface ParsedTOC {
	title: string;
	version: string | null;
	author: string | null;
	interface: string | null;
	requiredDeps: string[];
	optionalDeps: string[];
	xLibrary: boolean;
}

/**
 * Parses a TOC file content and extracts metadata.
 */
export function parseTOCContent(
	content: string,
	fallbackTitle: string,
): ParsedTOC {
	const titleMatch = content.match(/^## Title:\s*(.*)/m);
	const versionMatch = content.match(/^## Version:\s*(.*)/m);
	const authorMatch = content.match(/^## Author:\s*(.*)/m);
	const interfaceMatch = content.match(/^## Interface:\s*(.*)/m);
	const depsMatch = content.match(/^## (?:Dependencies|RequiredDeps):\s*(.*)/m);
	const optDepsMatch = content.match(/^## OptionalDeps:\s*(.*)/m);
	const xLibraryMatch = content.match(/^## X-Library:\s*(.*)/im);

	const title = titleMatch?.[1]?.trim() ?? fallbackTitle;
	const cleanTitle = title.replace(/\|c[0-9a-fA-F]{8}(.*?)\|r/g, "$1");

	const parseDeps = (raw: string | undefined): string[] => {
		if (!raw) return [];
		return raw.split(/,\s*|\s+/).filter(Boolean);
	};

	return {
		title: cleanTitle,
		version: versionMatch?.[1]?.trim() ?? null,
		author: authorMatch?.[1]?.trim() ?? null,
		interface: interfaceMatch?.[1]?.trim() ?? null,
		requiredDeps: parseDeps(depsMatch?.[1]),
		optionalDeps: parseDeps(optDepsMatch?.[1]),
		xLibrary: xLibraryMatch?.[1]?.trim().toLowerCase() === "true",
	};
}

export class ScanCommand implements Command<number> {
	private targetFlavor: GameFlavor = "retail";

	constructor(
		private dbManager: DatabaseManager,
		private configManager: ConfigManager,
		private specificFolders?: string[],
	) {}

	async execute(context: CommandContext): Promise<number> {
		context.emit("scan:start");
		const addonsDir = this.configManager.get().destDir;
		let exists = false;
		try {
			await fs.access(addonsDir);
			exists = true;
		} catch {
			exists = false;
		}

		if (!exists) {
			logger.log("Manager", `Addons directory not found: ${addonsDir}`);
			context.emit("scan:complete", 0);
			return 0;
		}

		// First pass: collect all TOC files grouped by folder
		const tocGlob = new Bun.Glob("*/*.toc");
		const folderTocs = new Map<string, string[]>();

		for await (const file of tocGlob.scan({ cwd: addonsDir })) {
			const folderName = path.dirname(file);
			const tocName = path.basename(file);
			if (!folderName || folderName === ".") continue;

			if (this.specificFolders && !this.specificFolders.includes(folderName)) {
				continue;
			}

			const existing = folderTocs.get(folderName) || [];
			existing.push(tocName);
			folderTocs.set(folderName, existing);
		}

		let count = 0;
		const scannedFolders: string[] = [];

		// Second pass: process each folder with best TOC selection
		for (const [folderName, tocFiles] of folderTocs) {
			context.emit("scan:progress", folderName);
			scannedFolders.push(folderName);

			try {
				// Select best TOC for target flavor
				const { selected: selectedToc } = selectTocFile(
					folderName,
					tocFiles,
					this.targetFlavor,
				);

				const fullPath = path.join(addonsDir, folderName, selectedToc);
				const content = await Bun.file(fullPath).text();

				// Parse TOC content
				const toc = parseTOCContent(content, folderName);

				// Detect embedded libraries
				const addonPath = path.join(addonsDir, folderName);
				const embeddedLibs = await detectEmbeddedLibs(addonPath);

				// Check for git repo
				const gitPath = path.join(addonPath, ".git");
				let isGit = false;
				let gitHash: string | null = null;
				try {
					await fs.stat(gitPath);
					isGit = true;
					gitHash = await GitClient.getCurrentCommit(addonPath);
				} catch {
					isGit = false;
				}

				const finalVersion =
					toc.version ||
					(isGit && gitHash ? gitHash.substring(0, 7) : "Unknown");

				const existing = this.dbManager.getByFolder(folderName);

				if (existing) {
					const updates: Partial<AddonRecord> = {};
					let updated = false;

					if (isGit && existing.type !== "github") {
						updates.type = "github";
						updated = true;
					}

					if (finalVersion && existing.version !== finalVersion) {
						updates.version = finalVersion;
						updated = true;
					}

					if (gitHash && existing.git_commit !== gitHash) {
						updates.git_commit = gitHash;
						updated = true;
					}

					if (toc.author && existing.author !== toc.author) {
						updates.author = toc.author;
						updated = true;
					}

					if (toc.interface && existing.interface !== toc.interface) {
						updates.interface = toc.interface;
						updated = true;
					}

					// Update dependencies and embedded libs
					if (
						JSON.stringify(toc.requiredDeps) !==
						JSON.stringify(existing.requiredDeps)
					) {
						updates.requiredDeps = toc.requiredDeps;
						updated = true;
					}

					if (
						JSON.stringify(toc.optionalDeps) !==
						JSON.stringify(existing.optionalDeps)
					) {
						updates.optionalDeps = toc.optionalDeps;
						updated = true;
					}

					if (
						JSON.stringify(embeddedLibs) !==
						JSON.stringify(existing.embeddedLibs)
					) {
						updates.embeddedLibs = embeddedLibs;
						updated = true;
					}

					if (updated) {
						this.dbManager.updateAddon(folderName, updates);
					}
				} else {
					this.dbManager.addAddon({
						name: toc.title,
						folder: folderName,
						version: finalVersion,
						git_commit: gitHash,
						author: toc.author,
						interface: toc.interface,
						url: null,
						type: "manual",
						ownedFolders: [],
						kind: "addon",
						kindOverride: false,
						flavor: this.targetFlavor,
						requiredDeps: toc.requiredDeps,
						optionalDeps: toc.optionalDeps,
						embeddedLibs: embeddedLibs,
						install_date: new Date().toISOString(),
						last_updated: new Date().toISOString(),
					});
					count++;
				}
			} catch (e) {
				context.emit(
					"error",
					`Scan:${folderName}`,
					e instanceof Error ? e.message : String(e),
				);
			}
		}

		// Post-scan: Auto-classify libraries based on heuristics
		this.classifyLibraries(scannedFolders);

		context.emit("scan:complete", count);
		return count;
	}

	/**
	 * Post-scan library classification using weighted heuristics.
	 * Only updates addons that haven't been manually classified.
	 */
	private classifyLibraries(folders: string[]): void {
		for (const folder of folders) {
			const addon = this.dbManager.getByFolder(folder);
			if (!addon) continue;

			// Skip if user has manually set the kind
			if (addon.kindOverride) continue;

			// Get dependency info for heuristics
			const dependents = this.dbManager.getDependents(folder);
			const hasDependents = dependents.length > 0;
			const hasDependencies =
				addon.requiredDeps.length > 0 || addon.optionalDeps.length > 0;

			// Run detection
			const result = detectLibraryKind(
				folder,
				{ xLibrary: undefined }, // TOC xLibrary parsed separately in scan
				{ hasDependents, hasDependencies },
			);

			// Only update if detected as library with at least medium confidence
			if (
				result.kind === "library" &&
				result.confidence !== "low" &&
				addon.kind !== "library"
			) {
				this.dbManager.updateAddon(folder, { kind: "library" });
				logger.log(
					"ScanCommand",
					`Classified ${folder} as library: ${result.reason}`,
				);
			}
		}
	}
}
