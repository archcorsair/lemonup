import fs from "node:fs";
import os from "node:os";
import { z } from "zod";
import type { AddonRecord } from "./db";

export const ExportedAddonSchema = z.object({
  name: z.string(),
  folder: z.string(),
  type: z.enum(["github", "tukui", "wowinterface", "manual"]),
  url: z.string().nullable(),
  ownedFolders: z.array(z.string()).optional(),
  reinstallable: z.boolean(),
});

export type ExportedAddon = z.infer<typeof ExportedAddonSchema>;

export const ExportFileSchema = z.object({
  version: z.literal(1),
  exportedAt: z.string(),
  addons: z.array(ExportedAddonSchema),
});

export type ExportFile = z.infer<typeof ExportFileSchema>;

export const DEFAULT_EXPORT_PATH = `${os.homedir()}/lemonup-addons.json`;

export async function exportAddons(
  addons: AddonRecord[],
  outputPath: string = DEFAULT_EXPORT_PATH,
): Promise<{ success: boolean; count: number; error?: string }> {
  try {
    const exported: ExportedAddon[] = addons.map((addon) => ({
      name: addon.name,
      folder: addon.folder,
      type: addon.type,
      url: addon.url,
      ownedFolders:
        addon.ownedFolders.length > 0 ? addon.ownedFolders : undefined,
      reinstallable: addon.type !== "manual" && !!addon.url,
    }));

    const exportFile: ExportFile = {
      version: 1,
      exportedAt: new Date().toISOString(),
      addons: exported,
    };

    await Bun.write(outputPath, JSON.stringify(exportFile, null, 2));

    return { success: true, count: exported.length };
  } catch (e) {
    return {
      success: false,
      count: 0,
      error: e instanceof Error ? e.message : String(e),
    };
  }
}

export async function parseImportFile(
  filePath: string,
): Promise<{ success: boolean; data?: ExportFile; error?: string }> {
  try {
    if (!fs.existsSync(filePath)) {
      return { success: false, error: `File not found: ${filePath}` };
    }

    const content = await Bun.file(filePath).text();
    const parsed = JSON.parse(content);
    const validated = ExportFileSchema.parse(parsed);

    return { success: true, data: validated };
  } catch (e) {
    if (e instanceof SyntaxError) {
      return { success: false, error: "Failed to parse JSON" };
    }
    if (e instanceof z.ZodError) {
      return {
        success: false,
        error: `Invalid format: ${e.issues[0]?.message}`,
      };
    }
    return {
      success: false,
      error: e instanceof Error ? e.message : String(e),
    };
  }
}

export interface ImportAnalysis {
  toInstall: ExportedAddon[];
  alreadyInstalled: ExportedAddon[];
  manualAddons: ExportedAddon[];
}

export function analyzeImport(
  exportData: ExportFile,
  currentAddons: AddonRecord[],
): ImportAnalysis {
  const installedFolders = new Set(
    currentAddons.map((a) => a.folder.toLowerCase()),
  );

  const toInstall: ExportedAddon[] = [];
  const alreadyInstalled: ExportedAddon[] = [];
  const manualAddons: ExportedAddon[] = [];

  for (const addon of exportData.addons) {
    if (!addon.reinstallable) {
      manualAddons.push(addon);
    } else if (installedFolders.has(addon.folder.toLowerCase())) {
      alreadyInstalled.push(addon);
    } else {
      toInstall.push(addon);
    }
  }

  return { toInstall, alreadyInstalled, manualAddons };
}
