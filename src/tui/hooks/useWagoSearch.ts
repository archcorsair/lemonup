import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
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

function useDebounce<T>(value: T, delay: number): T {
  const [debouncedValue, setDebouncedValue] = useState(value);

  useEffect(() => {
    const handler = setTimeout(() => {
      setDebouncedValue(value);
    }, delay);

    return () => {
      clearTimeout(handler);
    };
  }, [value, delay]);

  return debouncedValue;
}

export function useWagoSearch(
  options: UseWagoSearchOptions,
): UseWagoSearchReturn {
  const { debounceMs = 400, apiKey, searchFn = defaultSearchAddons } = options;

  const [query, setQuery] = useState("");
  const [gameVersion, setGameVersion] = useState<WagoGameVersion>("retail");
  const [stability, setStability] = useState<WagoStability>("stable");

  const debouncedQuery = useDebounce(query, debounceMs);

  const {
    data: results = [],
    isFetching,
    error: queryError,
    refetch: queryRefetch,
  } = useQuery({
    queryKey: ["wago", "search", debouncedQuery, gameVersion, stability],
    queryFn: async () => {
      if (!apiKey) {
        throw new Error("Wago API key not configured");
      }
      if (!debouncedQuery.trim()) {
        return [];
      }

      const response = await searchFn(
        debouncedQuery,
        gameVersion,
        apiKey,
        stability,
      );

      if (!response) {
        throw new Error("Search failed");
      }

      return response.data.map((addon) => ({
        ...addon,
        releases: addon.releases ?? { stable: undefined },
      })) as WagoAddonSummary[];
    },
    enabled: !!debouncedQuery.trim() && !!apiKey,
    placeholderData: keepPreviousData,
    staleTime: 5 * 60 * 1000,
  });

  const refetch = useCallback(() => {
    queryRefetch();
  }, [queryRefetch]);

  const finalResults = !debouncedQuery.trim() ? [] : results;

  let error: string | null = null;
  if (!apiKey) {
    if (query.trim()) error = "Wago API key not configured";
  } else if (queryError) {
    error = queryError instanceof Error ? queryError.message : "Search failed";
  }

  return {
    query,
    setQuery,
    results: finalResults,
    isLoading: isFetching,
    error,
    gameVersion,
    setGameVersion,
    stability,
    setStability,
    refetch,
  };
}
