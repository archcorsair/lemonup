import os from "node:os";
import { useEffect, useState } from "react";

interface ScanPathInput {
  inputValue: string;
  selectedIndex: number;
  suggestions: string[];
  setInputValue: (value: string) => void;
  selectIndex: (index: number) => void;
  getSelectedPath: () => string;
}

export function useScanPathInput(): ScanPathInput {
  const [inputValue, setInputValue] = useState(os.homedir());
  const [selectedIndex, setSelectedIndex] = useState(-1);
  const [suggestions, setSuggestions] = useState<string[]>([]);

  useEffect(() => {
    const platform = os.platform();
    const baseSuggestions: string[] = [os.homedir()];

    if (platform === "win32") {
      baseSuggestions.push("C:\\", "D:\\", "E:\\", "F:\\");
    } else if (platform === "linux") {
      baseSuggestions.push("/", "/home");
    } else if (platform === "darwin") {
      baseSuggestions.push("/Applications", "/");
    }

    setSuggestions(baseSuggestions);
  }, []);

  const getSelectedPath = (): string => {
    if (selectedIndex >= 0 && suggestions[selectedIndex]) {
      return suggestions[selectedIndex];
    }
    return inputValue || os.homedir();
  };

  return {
    inputValue,
    selectedIndex,
    suggestions,
    setInputValue,
    selectIndex: setSelectedIndex,
    getSelectedPath,
  };
}
