import { describe, expect, test } from "bun:test";
import { Text } from "ink";
import { render } from "ink-testing-library";
import { VirtualList } from "@/tui/components/VirtualList";

describe("VirtualList", () => {
  const items = ["Item 1", "Item 2", "Item 3", "Item 4", "Item 5"];

  test("renders visible items within viewport", () => {
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={3}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("Item 1");
    expect(frame).toContain("Item 2");
    expect(frame).toContain("Item 3");
    expect(frame).not.toContain("Item 4");
    expect(frame).not.toContain("Item 5");
  });

  test("shows overflow indicator when items below viewport", () => {
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={3}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("▼");
    expect(frame).toContain("2 more");
  });

  test("scrolls viewport to keep selected item visible", () => {
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={3}
        selectedIndex={4}
        renderItem={({ item, isSelected }) => (
          <Text inverse={isSelected}>{item}</Text>
        )}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("Item 5");
    expect(frame).toContain("▲");
  });

  test("marks selected item in renderItem props", () => {
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={5}
        selectedIndex={2}
        renderItem={({ item, isSelected }) => (
          <Text>{isSelected ? `[${item}]` : item}</Text>
        )}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("[Item 3]");
    expect(frame).not.toContain("[Item 1]");
  });

  test("renders all items when list fits in viewport", () => {
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={10}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("Item 1");
    expect(frame).toContain("Item 5");
    expect(frame).not.toContain("▼");
    expect(frame).not.toContain("▲");
  });

  test("uses custom keyExtractor when provided", () => {
    const objectItems = [
      { id: "a", name: "Alpha" },
      { id: "b", name: "Beta" },
    ];

    const { lastFrame } = render(
      <VirtualList
        items={objectItems}
        height={5}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => <Text>{item.name}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("Alpha");
    expect(frame).toContain("Beta");
  });
});

describe("VirtualList edge cases", () => {
  test("handles empty items array", () => {
    const { lastFrame } = render(
      <VirtualList
        items={[]}
        height={5}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).not.toContain("▼");
    expect(frame).not.toContain("▲");
  });

  test("handles single item", () => {
    const { lastFrame } = render(
      <VirtualList
        items={["Only one"]}
        height={5}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).toContain("Only one");
    expect(frame).not.toContain("▼");
  });

  test("clamps selectedIndex to valid range", () => {
    const { lastFrame } = render(
      <VirtualList
        items={["A", "B", "C"]}
        height={5}
        selectedIndex={100}
        renderItem={({ item, isSelected }) => (
          <Text>{isSelected ? `[${item}]` : item}</Text>
        )}
      />,
    );

    const frame = lastFrame() ?? "";
    // Should clamp to last item
    expect(frame).toContain("[C]");
  });

  test("handles negative selectedIndex", () => {
    const { lastFrame } = render(
      <VirtualList
        items={["A", "B", "C"]}
        height={5}
        selectedIndex={-5}
        renderItem={({ item, isSelected }) => (
          <Text>{isSelected ? `[${item}]` : item}</Text>
        )}
      />,
    );

    const frame = lastFrame() ?? "";
    // Should clamp to first item
    expect(frame).toContain("[A]");
  });

  test("hides overflow indicators when disabled", () => {
    const items = ["1", "2", "3", "4", "5"];
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={2}
        showOverflowIndicators={false}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    expect(frame).not.toContain("▼");
    expect(frame).not.toContain("▲");
  });

  test("uses custom overflow renderers", () => {
    const items = ["1", "2", "3", "4", "5"];
    const { lastFrame } = render(
      <VirtualList
        items={items}
        height={3}
        selectedIndex={4}
        renderOverflowTop={(n) => <Text>↑ {n} hidden</Text>}
        renderOverflowBottom={(n) => <Text>↓ {n} hidden</Text>}
        renderItem={({ item }) => <Text>{item}</Text>}
      />,
    );

    const frame = lastFrame() ?? "";
    // selectedIndex=4 scrolls viewport to show items 3,4,5 (indices 2,3,4)
    // with 2 items hidden above (indices 0,1) and 0 below
    expect(frame).toContain("↑ 2 hidden");
    expect(frame).not.toContain("↓");
  });
});
