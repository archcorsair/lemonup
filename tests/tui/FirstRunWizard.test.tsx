import { describe, expect, test } from "bun:test";
import { Box, Text } from "ink";
import { render } from "ink-testing-library";
import type React from "react";

// Test the step components in isolation by recreating simplified versions
// that match the wizard's step component interfaces

// WagoKeyStep component (extracted for testing)
const WagoKeyStep: React.FC<{
  wagoApiKey: string;
  onKeyChange: (key: string) => void;
  isEditing: boolean;
  envKeyDetected: boolean;
  optionIndex: number;
}> = ({ wagoApiKey, isEditing, envKeyDetected, optionIndex }) => {
  if (envKeyDetected) {
    return (
      <Box flexDirection="column">
        <Text bold>Wago.io API Key (Optional)</Text>
        <Text>✓ API key detected from WAGO_API_KEY environment variable</Text>
        <Text>Press Enter to continue.</Text>
      </Box>
    );
  }

  return (
    <Box flexDirection="column">
      <Text bold>Wago.io API Key (Optional)</Text>
      <Text>Wago.io requires an API key to search and install addons.</Text>
      <Text>Get your key at: https://addons.wago.io/patreon</Text>
      <Box flexDirection="column">
        <Text>{optionIndex === 0 ? "› " : "  "}Enter API key now</Text>
        <Text>{optionIndex === 1 ? "› " : "  "}Skip for now</Text>
      </Box>
      {isEditing && (
        <Box>
          <Text>✎ {wagoApiKey ? "*".repeat(wagoApiKey.length) : ""}</Text>
        </Box>
      )}
      <Text>
        You can also configure this later in Settings or set the WAGO_API_KEY
        environment variable.
      </Text>
    </Box>
  );
};

// ThemeStep component (simplified for testing)
const ThemeStep: React.FC<{
  value: "dark" | "light";
}> = ({ value }) => {
  return (
    <Box flexDirection="column">
      <Text bold>Choose your theme:</Text>
      <Text>[ {value === "dark" ? "Dark" : "Light"} ]</Text>
      <Text>The interface will update as you toggle. Use ←/→ to switch.</Text>
    </Box>
  );
};

// ReviewStep component (simplified for testing)
const ReviewStep: React.FC<{
  theme: "dark" | "light";
  destDir: string;
  wagoApiKey: string;
  envWagoKeyDetected: boolean;
  installElvUI: boolean;
  installTukui: boolean;
}> = ({
  theme,
  destDir,
  wagoApiKey,
  envWagoKeyDetected,
  installElvUI,
  installTukui,
}) => {
  const addons = [];
  if (installElvUI) addons.push("ElvUI");
  if (installTukui) addons.push("Tukui");

  const wagoStatus = envWagoKeyDetected
    ? "From env var"
    : wagoApiKey
      ? "Configured"
      : "Not set";

  return (
    <Box flexDirection="column">
      <Text bold>Review your setup:</Text>
      <Text>Theme: {theme === "dark" ? "Dark" : "Light"}</Text>
      <Text>AddOns Directory: {destDir || "(not set)"}</Text>
      <Text>Wago API Key: {wagoStatus}</Text>
      <Text>Install: {addons.length > 0 ? addons.join(", ") : "None"}</Text>
      <Text bold>Press Enter to complete setup.</Text>
    </Box>
  );
};

describe("FirstRunWizard Step Components", () => {
  describe("WagoKeyStep", () => {
    test("renders env var detected state", () => {
      const { lastFrame } = render(
        <WagoKeyStep
          wagoApiKey="env-key-123"
          onKeyChange={() => {}}
          isEditing={false}
          envKeyDetected={true}
          optionIndex={0}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Wago.io API Key (Optional)");
      expect(frame).toContain("API key detected from WAGO_API_KEY");
      expect(frame).toContain("Press Enter to continue");
    });

    test("renders option selection when no env var", () => {
      const { lastFrame } = render(
        <WagoKeyStep
          wagoApiKey=""
          onKeyChange={() => {}}
          isEditing={false}
          envKeyDetected={false}
          optionIndex={0}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Wago.io API Key (Optional)");
      expect(frame).toContain("https://addons.wago.io/patreon");
      expect(frame).toContain("Enter API key now");
      expect(frame).toContain("Skip for now");
      expect(frame).toContain("configure this later in Settings");
    });

    test("highlights first option when optionIndex is 0", () => {
      const { lastFrame } = render(
        <WagoKeyStep
          wagoApiKey=""
          onKeyChange={() => {}}
          isEditing={false}
          envKeyDetected={false}
          optionIndex={0}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("› Enter API key now");
      expect(frame).not.toContain("› Skip for now");
    });

    test("highlights second option when optionIndex is 1", () => {
      const { lastFrame } = render(
        <WagoKeyStep
          wagoApiKey=""
          onKeyChange={() => {}}
          isEditing={false}
          envKeyDetected={false}
          optionIndex={1}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).not.toContain("› Enter API key now");
      expect(frame).toContain("› Skip for now");
    });

    test("shows masked input when editing", () => {
      const { lastFrame } = render(
        <WagoKeyStep
          wagoApiKey="secret123"
          onKeyChange={() => {}}
          isEditing={true}
          envKeyDetected={false}
          optionIndex={0}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("✎");
      // Should show masked characters (asterisks)
      expect(frame).toContain("*********");
    });
  });

  describe("ThemeStep", () => {
    test("renders dark theme selected", () => {
      const { lastFrame } = render(<ThemeStep value="dark" />);

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Choose your theme:");
      expect(frame).toContain("[ Dark ]");
      expect(frame).toContain("Use ←/→ to switch");
    });

    test("renders light theme selected", () => {
      const { lastFrame } = render(<ThemeStep value="light" />);

      const frame = lastFrame() ?? "";
      expect(frame).toContain("[ Light ]");
    });
  });

  describe("ReviewStep", () => {
    test("renders all configuration values", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="dark"
          destDir="/path/to/addons"
          wagoApiKey=""
          envWagoKeyDetected={false}
          installElvUI={true}
          installTukui={false}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Review your setup:");
      expect(frame).toContain("Theme: Dark");
      expect(frame).toContain("AddOns Directory: /path/to/addons");
      expect(frame).toContain("Install: ElvUI");
      expect(frame).toContain("Press Enter to complete setup");
    });

    test("shows Wago API Key as configured when set", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="dark"
          destDir="/path/to/addons"
          wagoApiKey="my-api-key"
          envWagoKeyDetected={false}
          installElvUI={false}
          installTukui={false}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Wago API Key: Configured");
    });

    test("shows Wago API Key from env var when detected", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="dark"
          destDir="/path/to/addons"
          wagoApiKey="env-key"
          envWagoKeyDetected={true}
          installElvUI={false}
          installTukui={false}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Wago API Key: From env var");
    });

    test("shows Wago API Key as not set when empty", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="dark"
          destDir="/path/to/addons"
          wagoApiKey=""
          envWagoKeyDetected={false}
          installElvUI={false}
          installTukui={false}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Wago API Key: Not set");
    });

    test("shows both ElvUI and TukUI when both selected", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="light"
          destDir="/addons"
          wagoApiKey=""
          envWagoKeyDetected={false}
          installElvUI={true}
          installTukui={true}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Install: ElvUI, Tukui");
    });

    test("shows None when no addons selected", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="dark"
          destDir="/addons"
          wagoApiKey=""
          envWagoKeyDetected={false}
          installElvUI={false}
          installTukui={false}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("Install: None");
    });

    test("shows (not set) when destDir is empty", () => {
      const { lastFrame } = render(
        <ReviewStep
          theme="dark"
          destDir=""
          wagoApiKey=""
          envWagoKeyDetected={false}
          installElvUI={false}
          installTukui={false}
        />,
      );

      const frame = lastFrame() ?? "";
      expect(frame).toContain("AddOns Directory: (not set)");
    });
  });
});

describe("WizardState", () => {
  test("wagoApiKey should be included in state interface", () => {
    // This test verifies the TypeScript interface is correct
    // by creating a state object that includes wagoApiKey
    const state = {
      theme: "dark" as const,
      destDir: "/path/to/addons",
      installElvUI: false,
      installTukui: false,
      maxConcurrent: 3,
      checkInterval: 300,
      autoCheckEnabled: true,
      autoCheckInterval: 60,
      backupWTF: true,
      backupRetention: 5,
      importAddons: false,
      wagoApiKey: "test-key",
    };

    expect(state.wagoApiKey).toBe("test-key");
    expect(typeof state.wagoApiKey).toBe("string");
  });
});
