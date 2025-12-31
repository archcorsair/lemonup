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
