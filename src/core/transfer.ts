import { z } from "zod";

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
