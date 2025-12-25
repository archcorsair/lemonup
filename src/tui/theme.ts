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

  busy: "yellow",

  dryRun: "yellow.bold",

  testMode: "magenta",

  // Text Styles

  muted: "gray",

  highlight: "white.bold",

  // Specific UI Elements

  version: "gray",

  installed: "green",
  notInstalled: "gray",
} as const;
