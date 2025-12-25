/**
 * Theme Registry
 * Uses ink-color-pipe for styling.
 */

export const THEME = {
  // Core Identity
  brand: "cyan.bold",

  // Status Indicators
  success: "green.bold",
  error: "red.bold",
  warning: "yellow.bold",
  info: "blue",

  // Text Styles
  muted: "gray.dim",
  highlight: "white.bold",

  // Specific UI Elements
  keybinding: "gray",
  description: "white",

  // Addon List Styles
  addonName: "white.bold",
  version: "cyan",
  installed: "green",
  notInstalled: "gray",
} as const;
