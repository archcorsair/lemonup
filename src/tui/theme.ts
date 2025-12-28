/**
 * Theme Registry
 * Uses ink-color-pipe for styling.
 */

// Inspired by tokyonight-storm, thanks Folke!
export const DARK_THEME = {
  // Core Identity
  brand: "#7aa2f7.bold", // blue

  // Status Indicators
  success: "#9ece6a", // green
  error: "#db4b4b", // red1
  warning: "#e0af68", // yellow
  info: "#0db9d7", // blue2
  busy: "#bb9af7", // magenta
  dryRun: "#e0af68.bold", // yellow
  testMode: "#9d7cd8", // purple

  // Text Styles
  muted: "#565f89", // comment
  highlight: "#c0caf5.bold", // fg
  heading: "#7aa2f7.bold", // blue

  // Specific UI Elements
  version: "#737aa2", // dark5
  border: "#545c7e", // dark3
  keyActive: "#ff9e64.bold", // orange
  keyInactive: "#c0caf5", // fg
  labelActive: "#ff9e64", // orange
  labelInactive: "#a9b1d6", // fg_dark

  // Repository Types
  repoTukui: "#bb9af7", // magenta
  repoWowi: "#ff9e64", // orange
  repoManual: "#565f89", // comment
  repoGit: "#7aa2f7", // blue

  // Status Colors
  statusIdle: "#565f89", // comment
  statusWorking: "#7dcfff", // cyan
  statusChecking: "#e0af68", // yellow
  statusSuccess: "#9ece6a", // green
  statusUpToDate: "#9ece6a", // green
  statusWarning: "#e0af68", // yellow
  statusError: "#f7768e", // red

  // Help Panel
  helpKey: "#ff9e64.bold", // orange
  helpLabel: "#565f89", // comment

  // Tree & List
  treePrefix: "#545c7e", // dark3
  childText: "#737aa2", // dark5
  selection: "#7aa2f7", // blue
  checked: "#7dcfff", // cyan
  unchecked: "#565f89", // comment
  library: "#9d7cd8", // purple

  // Wizard
  wizardBorder: "#7aa2f7", // blue
  progressCompleted: "#9ece6a", // green
  progressCurrent: "#7aa2f7.bold", // blue
  progressPending: "#565f89", // comment
} as const;

// Derive the shape of the theme, but widen values to string
export type Theme = {
  [K in keyof typeof DARK_THEME]: string;
};

// Inspired by tokyonight-day
export const LIGHT_THEME: Theme = {
  ...DARK_THEME,
  brand: "#007197.bold", // cyan

  // Status Indicators
  success: "#587539", // green
  error: "#f52a65", // red
  warning: "#b15c00", // orange
  info: "#2e7de9", // blue
  busy: "#9854f1", // pink
  dryRun: "#b15c00.bold", // orange
  testMode: "#7847bd", // purple

  // Text Styles
  muted: "#848cb5", // comment
  highlight: "#3760bf.bold", // foreground
  heading: "#2e7de9.bold", // blue

  // Specific UI Elements
  version: "#848cb5", // comment
  border: "#2e7de9", // blue
  keyActive: "#b15c00.bold", // orange
  keyInactive: "#3760bf", // foreground
  labelActive: "#b15c00", // orange
  labelInactive: "#848cb5", // comment

  // Repository Types
  repoTukui: "#9854f1", // pink
  repoWowi: "#b15c00", // orange
  repoManual: "#848cb5", // comment
  repoGit: "#2e7de9", // blue

  // Status Colors
  statusIdle: "#848cb5", // comment
  statusWorking: "#007197", // cyan
  statusChecking: "#b15c00", // orange
  statusSuccess: "#587539", // green
  statusUpToDate: "#587539", // green
  statusWarning: "#b15c00", // orange
  statusError: "#f52a65", // red

  // Help Panel
  helpKey: "#b15c00.bold", // orange
  helpLabel: "#848cb5", // comment

  // Tree & List
  treePrefix: "#848cb5", // comment
  childText: "#848cb5", // comment
  selection: "#2e7de9", // blue
  checked: "#007197", // cyan
  unchecked: "#848cb5", // comment
  library: "#7847bd", // purple

  // Wizard
  wizardBorder: "#2e7de9", // blue
  progressCompleted: "#587539", // green
  progressCurrent: "#007197.bold", // cyan
  progressPending: "#848cb5", // comment
};

export const themes = {
  dark: DARK_THEME,
  light: LIGHT_THEME,
} as const;
// Legacy export for compatibility during refactor
export const THEME = DARK_THEME;
