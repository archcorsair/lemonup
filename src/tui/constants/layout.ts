/**
 * Static line heights for UI chrome elements.
 * Used to calculate available viewport space for scrollable content.
 */
export const LAYOUT = {
  /** Top + bottom border lines */
  BORDER: 2,

  /** App header (logo, status, etc.) */
  HEADER: 3,

  /** Screen title + subtitle/search area */
  SCREEN_TITLE: 2,

  /** Table column headers with border */
  COLUMN_HEADERS: 3,

  /** Bottom control bar */
  CONTROL_BAR: 2,

  /** Box padding={1} adds 1 line top + bottom */
  PADDING: 2,

  /** Overflow indicators (top + bottom) */
  OVERFLOW_INDICATORS: 2,
} as const;

/** Total chrome lines for ManageScreen */
export const MANAGE_SCREEN_RESERVED =
  LAYOUT.BORDER +
  LAYOUT.HEADER +
  LAYOUT.SCREEN_TITLE +
  LAYOUT.COLUMN_HEADERS +
  LAYOUT.CONTROL_BAR +
  LAYOUT.PADDING +
  LAYOUT.OVERFLOW_INDICATORS;

/** Chrome lines for screens without column headers */
export const SIMPLE_SCREEN_RESERVED =
  LAYOUT.BORDER +
  LAYOUT.HEADER +
  LAYOUT.SCREEN_TITLE +
  LAYOUT.CONTROL_BAR +
  LAYOUT.PADDING +
  LAYOUT.OVERFLOW_INDICATORS;
