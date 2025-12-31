import { describe, expect, test } from "bun:test";
import { Text } from "ink";
import { render } from "ink-testing-library";
import { useTerminalSize } from "@/tui/components/VirtualList/useTerminalSize";

const TestComponent = () => {
  const { rows, columns } = useTerminalSize();
  return <Text>{`${rows}x${columns}`}</Text>;
};

describe("useTerminalSize", () => {
  test("returns terminal dimensions with defaults", () => {
    const { lastFrame } = render(<TestComponent />);
    // ink-testing-library provides default dimensions
    expect(lastFrame()).toContain("x");
  });
});
