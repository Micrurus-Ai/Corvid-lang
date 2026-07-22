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

  const loadMore = useCallback(async (): Promise<void> => {
    if (inFlight.current || !hasMore) return;
    inFlight.current = true;
    setLoading(true);
    setError(null);
    try {
      const page = await fetchPage(cursor.current);
      setItems((prev) => [...prev, ...page.items]);
      cursor.current = page.next_cursor ?? undefined;
      setHasMore(page.has_more && page.next_cursor != null);
    } catch (err) {
      setError(err instanceof CorvidError ? err : new CorvidError(0, err, String(err)));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, [fetchPage, hasMore]);

  const reset = useCallback(() => {
    cursor.current = undefined;
    setItems([]);
    setHasMore(true);
    setError(null);
  }, []);

  // Load the first page on mount.
  useEffect(() => {
    void loadMore();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { items, loadMore, hasMore, loading, error, reset };
}
