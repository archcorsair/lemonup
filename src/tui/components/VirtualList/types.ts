import type { ReactNode } from "react";

export interface RenderItemProps<T> {
  item: T;
  index: number;
  isSelected: boolean;
}

export interface ViewportState {
  offset: number;
  visibleCount: number;
  totalCount: number;
}

export interface VirtualListProps<T> {
  // Data (required)
  items: T[];
  renderItem: (props: RenderItemProps<T>) => ReactNode;

  // Selection
  selectedIndex?: number;
  keyExtractor?: (item: T, index: number) => string;

  // Layout
  height?: number | "auto";
  reservedLines?: number;
  itemHeight?: number | ((item: T, index: number) => number);

  // Overflow Indicators
  showOverflowIndicators?: boolean;
  renderOverflowTop?: (count: number) => ReactNode;
  renderOverflowBottom?: (count: number) => ReactNode;

  // Future: Scrollbar
  renderScrollBar?: (viewport: ViewportState) => ReactNode;

  // Callbacks
  onViewportChange?: (viewport: ViewportState) => void;
}

export interface VirtualListRef {
  scrollToIndex: (
    index: number,
    alignment?: "auto" | "top" | "center" | "bottom",
  ) => void;
  getViewport: () => ViewportState;
  remeasure: () => void;
}
