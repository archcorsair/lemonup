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
  brand: "blue.bold",

  // Status Indicators
  success: "green",
  error: "red",
  warning: "magenta",
  info: "blue",
  busy: "magenta",
  dryRun: "magenta.bold",
  testMode: "blue",

  // Text Styles
  muted: "black.dim",
  highlight: "black.bold",
  heading: "blue.bold",

  // Specific UI Elements
  version: "black.dim",
  border: "black",
  keyActive: "blue.bold",
  keyInactive: "black",
  labelActive: "blue",
  labelInactive: "black",

  // Repository Types
  repoTukui: "magenta",
  repoWowi: "red",
  repoManual: "black.dim",
  repoGit: "blue",

  // Status Colors
  statusIdle: "black.dim",
  statusWorking: "blue",
  statusChecking: "magenta",
  statusSuccess: "green",
  statusWarning: "magenta",
  statusError: "red",

  // Help Panel
  helpKey: "blue.bold",
  helpLabel: "black",

  // Tree & List
  treePrefix: "black.dim",
  selection: "blue",
  checked: "green",
  unchecked: "black.dim",
  library: "magenta.dim",
};

export const themes = {
  dark: DARK_THEME,
  light: LIGHT_THEME,
} as const;
// Legacy export for compatibility during refactor
export const THEME = DARK_THEME;
