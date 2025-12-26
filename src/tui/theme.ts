/**
 * Theme Registry
 * Uses ink-color-pipe for styling.
 */

export const DARK_THEME = {
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
  statusUpToDate: "green",
  statusWarning: "yellow", // update available
  statusError: "red",

  // Help Panel
  helpKey: "yellow.bold",
  helpLabel: "gray",

  // Tree & List
  treePrefix: "gray",
  selection: "blue",
  checked: "green",
  unchecked: "gray",
  library: "cyan.dim",
} as const;

// Derive the shape of the theme, but widen values to string
export type Theme = {
  [K in keyof typeof DARK_THEME]: string;
};

export const LIGHT_THEME: Theme = {
  ...DARK_THEME,
  // Core Identity
  brand: "#0077aa.bold",

  // Status Indicators
  success: "#2e7d32",
  error: "#c62828",
  warning: "#bf5000",
  info: "#1565c0",
  busy: "#6a1b9a",
  dryRun: "#bf5000.bold",
  testMode: "#0077aa",

  // Text Styles
  muted: "#3d3d3d",
  highlight: "#1e3a5f.bold",
  heading: "#0077aa.bold",

  // Specific UI Elements
  version: "#555555",
  border: "#1e3a5f",
  keyActive: "#bf5000.bold",
  keyInactive: "#2a2a2a",
  labelActive: "#bf5000",
  labelInactive: "#3a3a3a",

  // Repository Types
  repoTukui: "#6a1b9a",
  repoWowi: "#bf360c",
  repoManual: "#555555",
  repoGit: "#1565c0",

  // Status Colors
  statusIdle: "#555555",
  statusWorking: "#0077aa",
  statusChecking: "#bf5000",
  statusSuccess: "#2e7d32",
  statusUpToDate: "#bf5000",
  statusWarning: "#bf5000",
  statusError: "#c62828",

  // Help Panel
  helpKey: "#bf5000.bold",
  helpLabel: "#3d3d3d",

  // Tree & List
  treePrefix: "#5a5a5a",
  selection: "#bf5000",
  checked: "#0077aa",
  unchecked: "#5a5a5a",
  library: "#6a1b9a",
};

export const themes = {
  dark: DARK_THEME,
  light: LIGHT_THEME,
} as const;
// Legacy export for compatibility during refactor
export const THEME = DARK_THEME;
