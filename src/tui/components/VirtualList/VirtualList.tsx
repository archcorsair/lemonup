import { Box, Text } from "ink";
import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useState,
} from "react";
import type {
  RenderItemProps,
  ViewportState,
  VirtualListProps,
  VirtualListRef,
} from "./types";
import { useTerminalSize } from "./useTerminalSize";

const DEFAULT_HEIGHT = 10;
const DEFAULT_ITEM_HEIGHT = 1;

function calculateViewportOffset(
  selectedIndex: number,
  currentOffset: number,
  visibleCount: number,
): number {
  // Selection above viewport - scroll up
  if (selectedIndex < currentOffset) {
    return selectedIndex;
  }

  // Selection below viewport - scroll down
  if (selectedIndex >= currentOffset + visibleCount) {
    return selectedIndex - visibleCount + 1;
  }

  // Selection visible - no change
  return currentOffset;
}

function VirtualListInner<T>(
  props: VirtualListProps<T>,
  ref: React.ForwardedRef<VirtualListRef>,
) {
  const {
    items,
    renderItem,
    selectedIndex = 0,
    keyExtractor,
    height = DEFAULT_HEIGHT,
    reservedLines = 0,
    itemHeight = DEFAULT_ITEM_HEIGHT,
    showOverflowIndicators = true,
    renderOverflowTop,
    renderOverflowBottom,
    renderScrollBar,
    onViewportChange,
  } = props;

  const { rows: terminalRows } = useTerminalSize();

  // Calculate resolved height
  const resolvedHeight = useMemo(() => {
    if (typeof height === "number") {
      return height;
    }
    // 'auto' - use terminal rows minus reserved
    return Math.max(1, terminalRows - reservedLines);
  }, [height, terminalRows, reservedLines]);

  // Calculate item height (static for now, dynamic support later)
  const staticItemHeight =
    typeof itemHeight === "number" ? itemHeight : DEFAULT_ITEM_HEIGHT;

  // Reserve space for overflow indicators within the height budget
  // This ensures total output (items + indicators) stays within height
  const indicatorLines = showOverflowIndicators ? 2 : 0;
  const availableHeight = Math.max(0, resolvedHeight - indicatorLines);

  // Calculate how many items fit in viewport
  const visibleCount = Math.floor(availableHeight / staticItemHeight);

  // Clamp selectedIndex to valid range
  const clampedSelectedIndex = Math.max(
    0,
    Math.min(selectedIndex, items.length - 1),
  );

  // Calculate viewport offset - use useMemo to derive from selectedIndex
  // This ensures the viewport is always in sync with selection
  const calculatedOffset = useMemo(() => {
    if (items.length === 0) return 0;

    const maxOffset = Math.max(0, items.length - visibleCount);

    // Calculate what offset would show the selected item
    let offset = 0;

    // Selection below viewport - scroll down
    if (clampedSelectedIndex >= visibleCount) {
      offset = clampedSelectedIndex - visibleCount + 1;
    }

    return Math.min(Math.max(0, offset), maxOffset);
  }, [items.length, visibleCount, clampedSelectedIndex]);

  const [viewportOffset, setViewportOffset] = useState(calculatedOffset);

  // Sync viewportOffset with calculatedOffset when selection changes
  useEffect(() => {
    // Check if selected item is outside current viewport
    if (clampedSelectedIndex < viewportOffset) {
      // Selection above viewport - scroll up
      setViewportOffset(clampedSelectedIndex);
    } else if (clampedSelectedIndex >= viewportOffset + visibleCount) {
      // Selection below viewport - scroll down
      const maxOffset = Math.max(0, items.length - visibleCount);
      const newOffset = clampedSelectedIndex - visibleCount + 1;
      setViewportOffset(Math.min(newOffset, maxOffset));
    }
  }, [clampedSelectedIndex, viewportOffset, visibleCount, items.length]);

  // Build viewport state
  const viewport: ViewportState = useMemo(
    () => ({
      offset: viewportOffset,
      visibleCount,
      totalCount: items.length,
    }),
    [viewportOffset, visibleCount, items.length],
  );

  // Notify on viewport change
  useEffect(() => {
    onViewportChange?.(viewport);
  }, [viewport, onViewportChange]);

  // Imperative handle
  useImperativeHandle(
    ref,
    () => ({
      scrollToIndex: (
        index: number,
        alignment: "auto" | "top" | "center" | "bottom" = "auto",
      ) => {
        const clampedIndex = Math.max(0, Math.min(index, items.length - 1));
        let newOffset: number;

        switch (alignment) {
          case "top":
            newOffset = clampedIndex;
            break;
          case "center":
            newOffset = clampedIndex - Math.floor(visibleCount / 2);
            break;
          case "bottom":
            newOffset = clampedIndex - visibleCount + 1;
            break;
          default: // 'auto'
            newOffset = calculateViewportOffset(
              clampedIndex,
              viewportOffset,
              visibleCount,
            );
        }

        const maxOffset = Math.max(0, items.length - visibleCount);
        setViewportOffset(Math.min(Math.max(0, newOffset), maxOffset));
      },
      getViewport: () => viewport,
      remeasure: () => {
        // Force recalculation by updating state
        setViewportOffset((prev) => {
          const maxOffset = Math.max(0, items.length - visibleCount);
          return Math.min(prev, maxOffset);
        });
      },
    }),
    [items.length, visibleCount, viewportOffset, viewport],
  );

  // Calculate overflow counts
  const overflowTop = viewportOffset;
  const overflowBottom = Math.max(
    0,
    items.length - viewportOffset - visibleCount,
  );

  // Get visible items
  const visibleItems = items.slice(
    viewportOffset,
    viewportOffset + visibleCount,
  );

  // Default overflow renderers
  const defaultOverflowTop = (count: number) => (
    <Text dimColor>▲ {count} more</Text>
  );
  const defaultOverflowBottom = (count: number) => (
    <Text dimColor>▼ {count} more</Text>
  );

  const topIndicator = renderOverflowTop ?? defaultOverflowTop;
  const bottomIndicator = renderOverflowBottom ?? defaultOverflowBottom;

  return (
    <Box flexDirection="column">
      {showOverflowIndicators && overflowTop > 0 && topIndicator(overflowTop)}

      {visibleItems.map((item, idx) => {
        const actualIndex = viewportOffset + idx;
        const key = keyExtractor
          ? keyExtractor(item, actualIndex)
          : String(actualIndex);

        const itemProps: RenderItemProps<T> = {
          item,
          index: actualIndex,
          isSelected: actualIndex === clampedSelectedIndex,
        };

        return <Box key={key}>{renderItem(itemProps)}</Box>;
      })}

      {showOverflowIndicators &&
        overflowBottom > 0 &&
        bottomIndicator(overflowBottom)}

      {renderScrollBar?.(viewport)}
    </Box>
  );
}

export const VirtualList = forwardRef(VirtualListInner) as <T>(
  props: VirtualListProps<T> & { ref?: React.ForwardedRef<VirtualListRef> },
) => ReturnType<typeof VirtualListInner>;
