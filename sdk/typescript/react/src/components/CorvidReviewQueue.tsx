// Human-review queue (slice 51p). A generic list of items awaiting a
// human decision (accept / reject) — the frontend face of a
// confidence-gated or approval-gated workflow. Paginate the source with
// `useCorvidPaginated` and feed its items here.

import { type ReactNode } from "react";

export interface CorvidReviewQueueProps<Item> {
  items: Item[];
  /** Stable key for an item. Defaults to the array index. */
  keyOf?: (item: Item, index: number) => string | number;
  /** Render an item's body. Defaults to pretty-printed JSON. */
  renderItem: (item: Item) => ReactNode;
  onAccept?: (item: Item) => void;
  onReject?: (item: Item) => void;
  /** Called when the list nears its end (wire to `loadMore`). */
  onLoadMore?: () => void;
  className?: string;
}

/** A review queue: one row per item with accept/reject actions. */
export function CorvidReviewQueue<Item>(props: CorvidReviewQueueProps<Item>): ReactNode {
  const { items, keyOf, renderItem, onAccept, onReject, onLoadMore, className } = props;
  return (
    <div className={className}>
      <ul style={{ listStyle: "none", padding: 0 }}>
        {items.map((item, i) => (
          <li key={keyOf ? keyOf(item, i) : i} className="corvid-review-item" style={{ display: "flex", gap: 8 }}>
            <div style={{ flex: 1 }}>{renderItem(item)}</div>
            {onAccept && <button onClick={() => onAccept(item)}>Accept</button>}
            {onReject && <button onClick={() => onReject(item)}>Reject</button>}
          </li>
        ))}
      </ul>
      {onLoadMore && (
        <button onClick={onLoadMore} className="corvid-load-more">
          Load more
        </button>
      )}
    </div>
  );
}
