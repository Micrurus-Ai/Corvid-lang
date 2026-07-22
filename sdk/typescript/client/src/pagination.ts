// Cursor pagination (slice 51l), mirroring `Page<Item>` (slice 51f).
// The generic `paginate` helper walks a cursor-paginated route to
// exhaustion so a caller writes `for await (const item of paginate(...))`.

/** One page of a cursor-paginated result. */
export interface Page<Item> {
  items: Item[];
  next_cursor: string | null;
  has_more: boolean;
}

/**
 * Walk every page of a cursor-paginated endpoint. `fetchPage` receives
 * the current cursor (undefined for the first page) and returns the
 * next `Page`; iteration stops when `has_more` is false.
 */
export async function* paginate<Item>(
  fetchPage: (cursor?: string) => Promise<Page<Item>>,
): AsyncGenerator<Item> {
  let cursor: string | undefined = undefined;
  for (;;) {
    const page = await fetchPage(cursor);
    for (const item of page.items) {
      yield item;
    }
    if (!page.has_more || !page.next_cursor) {
      return;
    }
    cursor = page.next_cursor;
  }
}
