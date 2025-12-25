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

  heading: "magenta.bold",

  // Specific UI Elements
  version: "gray",
  border: "gray",
  keyActive: "magenta.bold",
  keyInactive: "white.bold",
  labelActive: "magenta",
  labelInactive: "white",

  // Repository Types
  repoTukui: "magenta",
  repoWowi: "yellow",
  repoManual: "gray",
  repoGit: "blue",

  // Status Colors
  statusIdle: "gray",
  statusWorking: "cyan", // downloading, extracting, copying
  statusChecking: "yellow",
  statusSuccess: "green",
  statusWarning: "yellow", // update available
  statusError: "red",

  // Tree & List
  treePrefix: "gray",
  selection: "blue",
  checked: "green",
  unchecked: "gray",
  library: "cyan.dim",
} as const;
