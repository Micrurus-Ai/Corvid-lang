// React hook for cursor pagination (slice 51n), over `Page<Item>`
// (slice 51f). Generic over the item type; the generated method
// signature specializes it at the call site.

import { useCallback, useEffect, useRef, useState } from "react";
import { CorvidError, type Page } from "@corvid/client";

export interface PaginatedHandle<Item> {
  /** All items loaded so far, across every page fetched. */
  items: Item[];
  /** Load the next page. No-op while loading or once exhausted. */
  loadMore: () => Promise<void>;
  /** Whether another page exists. */
  hasMore: boolean;
  /** Whether a page fetch is in flight. */
  loading: boolean;
  /** The most recent error, or null. */
  error: CorvidError | null;
  /** Reset to the first page and refetch. */
  reset: () => void;
}

/**
 * Drive a cursor-paginated endpoint. Pass a bound method returning a
 * `Page<Item>`, e.g. `useCorvidPaginated((c) => api.browse(c ?? ""))`.
 * The first page loads on mount.
 */
export function useCorvidPaginated<Item>(
  fetchPage: (cursor?: string) => Promise<Page<Item>>,
): PaginatedHandle<Item> {
  const [items, setItems] = useState<Item[]>([]);
  const [hasMore, setHasMore] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<CorvidError | null>(null);
  const cursor = useRef<string | undefined>(undefined);
  const inFlight = useRef(false);

  const generation = useRef(0);
  const fetch = useCallback(async (first = false): Promise<void> => {
    if (inFlight.current || (!first && !hasMore)) return;
    const current = generation.current;
    inFlight.current = true;
    setLoading(true);
    setError(null);
    try {
      const page = await fetchPage(cursor.current);
      if (current !== generation.current) return;
      setItems((prev) => [...prev, ...page.items]);
      cursor.current = page.next_cursor ?? undefined;
      setHasMore(page.has_more && page.next_cursor != null);
    } catch (err) {
      if (current !== generation.current) return;
      setError(err instanceof CorvidError ? err : new CorvidError(0, err, String(err)));
    } finally {
      if (current === generation.current) {
        inFlight.current = false;
        setLoading(false);
      }
    }
  }, [fetchPage, hasMore]);

  const reset = useCallback(() => {
    generation.current++;
    inFlight.current = false;
    cursor.current = undefined;
    setItems([]);
    setHasMore(true);
    void fetch(true);
  }, [fetch]);

  // Load the first page on mount.
  useEffect(() => {
    void fetch(true);
    return () => { generation.current++; inFlight.current = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { items, loadMore: () => fetch(), hasMore, loading, error, reset };
}
