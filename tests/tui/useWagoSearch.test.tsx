import { beforeEach, describe, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Text } from "ink";
import { render } from "ink-testing-library";
import { useEffect, useState } from "react";
import type { WagoSearchResponse } from "@/core/wago";
import { useWagoSearch } from "@/tui/hooks/useWagoSearch";

// Mock search function with proper typing
type SearchAddonsMock = (
  query: string,
  gameVersion: string,
  apiKey?: string,
  stability?: string,
) => Promise<WagoSearchResponse | null>;

const mockSearchAddons = mock<SearchAddonsMock>(() =>
  Promise.resolve({ data: [] }),
);

// Test harness component that exposes hook state
interface TestHarnessProps {
  apiKey: string | undefined;
  debounceMs?: number;
  initialQuery?: string;
  initialGameVersion?: string;
  initialStability?: string;
  onStateChange?: (state: ReturnType<typeof useWagoSearch>) => void;
}

function TestHarness({
  apiKey,
  debounceMs,
  initialQuery,
  initialGameVersion,
  initialStability,
  onStateChange,
}: TestHarnessProps) {
  const hook = useWagoSearch({
    apiKey,
    debounceMs,
    searchFn: mockSearchAddons,
  });
  const [initialized, setInitialized] = useState(false);

  // Apply initial values
  useEffect(() => {
    if (!initialized) {
      if (initialQuery !== undefined) {
        hook.setQuery(initialQuery);
      }
      if (initialGameVersion !== undefined) {
        hook.setGameVersion(
          initialGameVersion as "retail" | "classic" | "cata",
        );
      }
      if (initialStability !== undefined) {
        hook.setStability(initialStability as "stable" | "beta" | "alpha");
      }
      setInitialized(true);
    }
  }, [initialized, initialQuery, initialGameVersion, initialStability, hook]);

  // Report state changes
  useEffect(() => {
    onStateChange?.(hook);
  }, [hook, onStateChange]);

  return (
    <Text>
      query:{hook.query}|results:{hook.results.length}|loading:
      {String(hook.isLoading)}|error:{hook.error ?? "null"}|gameVersion:
      {hook.gameVersion}|stability:{hook.stability}
    </Text>
  );
}

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: 0,
      },
    },
  });

  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function renderWithClient(ui: React.ReactElement) {
  const Wrapper = createWrapper();
  return render(<Wrapper>{ui}</Wrapper>);
}

describe("useWagoSearch", () => {
  beforeEach(() => {
    mockSearchAddons.mockClear();
    mockSearchAddons.mockResolvedValue({ data: [] });
  });

  describe("initialization", () => {
    test("initializes with empty state", async () => {
      const { lastFrame } = renderWithClient(<TestHarness apiKey="test-key" />);

      // Wait for debounce to settle
      await new Promise((r) => setTimeout(r, 600));

      const frame = lastFrame() ?? "";
      expect(frame).toContain("query:");
      expect(frame).toContain("results:0");
      expect(frame).toContain("loading:false");
      expect(frame).toContain("error:null");
      expect(frame).toContain("gameVersion:retail");
      expect(frame).toContain("stability:stable");
    });

    test("initializes with custom debounce time", async () => {
      const { lastFrame } = renderWithClient(
        <TestHarness apiKey="test-key" debounceMs={200} />,
      );

      // Wait for debounce to settle
      await new Promise((r) => setTimeout(r, 400));

      const frame = lastFrame() ?? "";
      expect(frame).toContain("loading:false");
    });
  });

  describe("API key validation", () => {
    test("sets error when no API key provided and query is set", async () => {
      const { lastFrame } = renderWithClient(
        <TestHarness apiKey={undefined} initialQuery="test" />,
      );

      // Wait for debounce (400ms default + buffer)
      await new Promise((r) => setTimeout(r, 600));

      const frame = lastFrame() ?? "";
      // Error text may span multiple lines in Ink rendering, check key parts
      expect(frame).toContain("Wago API key");
      expect(frame).toContain("configured");
      expect(frame).toContain("results:0");
    });
  });

  describe("query handling", () => {
    test("clears results when query is empty", async () => {
      const { lastFrame } = renderWithClient(
        <TestHarness apiKey="test-key" initialQuery="" />,
      );

      await new Promise((r) => setTimeout(r, 600));

      const frame = lastFrame() ?? "";
      expect(frame).toContain("results:0");
      expect(frame).toContain("error:null");
    });

    test("trims whitespace-only query", async () => {
      const { lastFrame } = renderWithClient(
        <TestHarness apiKey="test-key" initialQuery="   " />,
      );

      await new Promise((r) => setTimeout(r, 600));

      const frame = lastFrame() ?? "";
      expect(frame).toContain("results:0");
      expect(mockSearchAddons).not.toHaveBeenCalled();
    });
  });

  describe("search execution", () => {
    test("calls searchAddons with correct parameters", async () => {
      mockSearchAddons.mockResolvedValueOnce({
        data: [
          {
            id: "test-addon",
            display_name: "Test Addon",
            summary: "Test summary",
            thumbnail_image: null,
          },
        ],
      });

      renderWithClient(<TestHarness apiKey="test-key" initialQuery="test" />);

      await new Promise((r) => setTimeout(r, 600));

      expect(mockSearchAddons).toHaveBeenCalledWith(
        "test",
        "retail",
        "test-key",
        "stable",
      );
    });

    test("updates results on successful search", async () => {
      mockSearchAddons.mockResolvedValueOnce({
        data: [
          {
            id: "addon-1",
            display_name: "Addon 1",
            summary: "Summary 1",
            thumbnail_image: null,
          },
          {
            id: "addon-2",
            display_name: "Addon 2",
            summary: "Summary 2",
            thumbnail_image: null,
          },
        ],
      });

      const { lastFrame } = renderWithClient(
        <TestHarness apiKey="test-key" initialQuery="elvui" />,
      );

      await new Promise((r) => setTimeout(r, 600));

      const frame = lastFrame() ?? "";
      expect(frame).toContain("results:2");
      expect(frame).toContain("error:null");
    });

    test("sets error when search returns null", async () => {
      mockSearchAddons.mockResolvedValueOnce(null);

      const { lastFrame } = renderWithClient(
        <TestHarness apiKey="test-key" initialQuery="test" />,
      );

      await new Promise((r) => setTimeout(r, 600));

      const frame = lastFrame() ?? "";
      expect(frame).toContain("error:Search failed");
      expect(frame).toContain("results:0");
    });
  });

  describe("filter changes", () => {
    test("uses provided game version", async () => {
      mockSearchAddons.mockResolvedValue({ data: [] });

      renderWithClient(
        <TestHarness
          apiKey="test-key"
          initialQuery="test"
          initialGameVersion="classic"
        />,
      );

      await new Promise((r) => setTimeout(r, 600));

      // Check that searchAddons was called with classic
      const calls = mockSearchAddons.mock.calls;
      const classicCall = calls.find((call) => call[1] === "classic");
      expect(classicCall).toBeDefined();
    });

    test("uses provided stability", async () => {
      mockSearchAddons.mockResolvedValue({ data: [] });

      renderWithClient(
        <TestHarness
          apiKey="test-key"
          initialQuery="test"
          initialStability="beta"
        />,
      );

      await new Promise((r) => setTimeout(r, 600));

      // Check that searchAddons was called with beta
      const calls = mockSearchAddons.mock.calls;
      const betaCall = calls.find((call) => call[3] === "beta");
      expect(betaCall).toBeDefined();
    });
  });

  describe("state exposure", () => {
    test("exposes all required state and methods", () => {
      let capturedState: ReturnType<typeof useWagoSearch> | undefined;

      renderWithClient(
        <TestHarness
          apiKey="test-key"
          onStateChange={(state) => {
            capturedState = state;
          }}
        />,
      );

      expect(capturedState).toBeDefined();
      if (capturedState) {
        expect(typeof capturedState.query).toBe("string");
        expect(typeof capturedState.setQuery).toBe("function");
        expect(Array.isArray(capturedState.results)).toBe(true);
        expect(typeof capturedState.isLoading).toBe("boolean");
        expect(typeof capturedState.gameVersion).toBe("string");
        expect(typeof capturedState.setGameVersion).toBe("function");
        expect(typeof capturedState.stability).toBe("string");
        expect(typeof capturedState.setStability).toBe("function");
        expect(typeof capturedState.refetch).toBe("function");
      }
    });
  });
});
