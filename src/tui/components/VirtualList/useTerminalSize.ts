import { useStdout } from "ink";
import { useEffect, useState } from "react";

export interface TerminalSize {
  rows: number;
  columns: number;
}

const DEFAULT_ROWS = 24;
const DEFAULT_COLUMNS = 80;

export function useTerminalSize(): TerminalSize {
  const { stdout } = useStdout();
  const [size, setSize] = useState<TerminalSize>({
    rows: stdout.rows ?? DEFAULT_ROWS,
    columns: stdout.columns ?? DEFAULT_COLUMNS,
  });

  useEffect(() => {
    const handleResize = () => {
      setSize({
        rows: stdout.rows ?? DEFAULT_ROWS,
        columns: stdout.columns ?? DEFAULT_COLUMNS,
      });
    };

    stdout.on("resize", handleResize);
    return () => {
      stdout.off("resize", handleResize);
    };
  }, [stdout]);

  return size;
}
