import { AsyncDebouncer } from "@tanstack/pacer";
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";
import {
  searchAddons as defaultSearchAddons,
  type WagoAddonSummary,
  type WagoGameVersion,
  type WagoSearchResponse,
  type WagoStability,
} from "@/core/wago";

type SearchAddonsFn = (
  query: string,
  gameVersion: WagoGameVersion,
  apiKey?: string,
  stability?: WagoStability,
) => Promise<WagoSearchResponse | null>;

interface UseWagoSearchOptions {
  debounceMs?: number;
  apiKey: string | undefined;
  /** Optional search function for testing - defaults to searchAddons from wago module */
  searchFn?: SearchAddonsFn;
}

interface UseWagoSearchReturn {
  query: string;
  setQuery: (q: string) => void;
  results: WagoAddonSummary[];
  isLoading: boolean;
  error: string | null;
  gameVersion: WagoGameVersion;
  setGameVersion: (v: WagoGameVersion) => void;
  stability: WagoStability;
  setStability: (s: WagoStability) => void;
  refetch: () => void;
}

export function useWagoSearch(
  options: UseWagoSearchOptions,
): UseWagoSearchReturn {
  const { debounceMs = 400, apiKey, searchFn = defaultSearchAddons } = options;

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<WagoAddonSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [gameVersion, setGameVersion] = useState<WagoGameVersion>("retail");
  const [stability, setStability] = useState<WagoStability>("stable");

  // Create AsyncDebouncer instance with TanStack Pacer
  const debouncer = useMemo(
    () =>
      new AsyncDebouncer(
        async (
          searchQuery: string,
          version: WagoGameVersion,
          stab: WagoStability,
          key: string | undefined,
        ) => {
          if (!key) {
            setError("Wago API key not configured");
            setResults([]);
            return null;
          }

          if (!searchQuery.trim()) {
            setResults([]);
            setError(null);
            return null;
          }

          setError(null);

          const response = await searchFn(searchQuery, version, key, stab);

          if (response) {
            // Map API response to WagoAddonSummary with required releases field
            const mappedResults = response.data.map((addon) => ({
              ...addon,
              releases: addon.releases ?? { stable: undefined },
            })) as WagoAddonSummary[];
            setResults(mappedResults);
            return mappedResults;
          }

          setError("Search failed");
          setResults([]);
          return null;
        },
        {
          wait: debounceMs,
          onError: (err) => {
            setError(err instanceof Error ? err.message : "Search failed");
            setResults([]);
          },
        },
      ),
    [debounceMs, searchFn],
  );

  // Subscribe to store state changes for loading indicator
  const debouncerState = useSyncExternalStore(
    (callback) => debouncer.store.subscribe(callback),
    () => debouncer.store.state,
  );

  const isLoading = debouncerState.isExecuting || debouncerState.isPending;

  // Trigger search when query or filters change
  useEffect(() => {
    debouncer.maybeExecute(query, gameVersion, stability, apiKey);
  }, [query, gameVersion, stability, apiKey, debouncer]);

  const refetch = useCallback(() => {
    if (query.trim()) {
      debouncer.maybeExecute(query, gameVersion, stability, apiKey);
    }
  }, [query, gameVersion, stability, apiKey, debouncer]);

  return {
    query,
    setQuery,
    results,
    isLoading,
    error,
    gameVersion,
    setGameVersion,
    stability,
    setStability,
    refetch,
  };
}
