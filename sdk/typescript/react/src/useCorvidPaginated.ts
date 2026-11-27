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
  fetchPage: (cursor?: string, signal?: AbortSignal) => Promise<Page<Item>>,
): PaginatedHandle<Item> {
  const [items, setItems] = useState<Item[]>([]);
  const [hasMore, setHasMore] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<CorvidError | null>(null);
  const cursor = useRef<string | undefined>(undefined);
  const inFlight = useRef<AbortController | null>(null);
  const exhausted = useRef(false);

  const loadMore = useCallback(async (): Promise<void> => {
    if (inFlight.current || exhausted.current) return;
    const request = new AbortController();
    inFlight.current = request;
    setLoading(true);
    setError(null);
    try {
      const page = await fetchPage(cursor.current, request.signal);
      if (request.signal.aborted) return;
      setItems((prev) => [...prev, ...page.items]);
      cursor.current = page.next_cursor ?? undefined;
      exhausted.current = !page.has_more || page.next_cursor == null;
      setHasMore(!exhausted.current);
    } catch (err) {
      if (!request.signal.aborted) {
        setError(err instanceof CorvidError ? err : new CorvidError(0, err, String(err)));
      }
    } finally {
      if (inFlight.current === request) {
        inFlight.current = null;
        setLoading(false);
      }
    }
  }, [fetchPage]);

  const reset = useCallback(() => {
    inFlight.current?.abort();
    inFlight.current = null;
    exhausted.current = false;
    cursor.current = undefined;
    setItems([]);
    setHasMore(true);
    setError(null);
    void loadMore();
  }, [loadMore]);

  // Load the first page on mount.
  useEffect(() => {
    reset();
    return () => inFlight.current?.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { items, loadMore, hasMore, loading, error, reset };
}
