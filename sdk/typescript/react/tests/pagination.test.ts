import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { act, create } from "react-test-renderer";
import type { Page } from "@corvid/client";
import { useCorvidPaginated, type PaginatedHandle } from "../src/useCorvidPaginated.js";

test("reset supersedes a pending page and failures preserve the retry cursor", async () => {
  const requests: Array<{
    cursor?: string;
    signal?: AbortSignal;
    resolve: (page: Page<string>) => void;
    reject: (error: Error) => void;
  }> = [];
  let state!: PaginatedHandle<string>;
  function Orders() {
    state = useCorvidPaginated<string>((cursor, signal) =>
      new Promise((resolve, reject) => requests.push({ cursor, signal, resolve, reject })));
    return null;
  }
  let component!: ReturnType<typeof create>;
  await act(() => { component = create(createElement(Orders)); });
  assert.equal(state.loading, true);
  await act(() => state.reset());
  assert.ok(requests[0].signal?.aborted);
  await act(() => requests[0].resolve({ items: ["obsolete"], next_cursor: null, has_more: false }));
  assert.deepEqual(state.items, []);
  assert.ok(state.loading);
  await act(() => requests[1].resolve({ items: ["first"], next_cursor: "second", has_more: true }));
  await act(() => { void state.loadMore(); });
  await act(() => requests[2].reject(new Error("connection interrupted")));
  assert.ok(state.error);
  assert.deepEqual(state.items, ["first"]);
  await act(() => { void state.loadMore(); });
  assert.equal(requests[2].cursor, "second");
  assert.equal(requests[3].cursor, "second");
  await act(() => requests[3].resolve({ items: ["last"], next_cursor: null, has_more: false }));
  assert.deepEqual(state.items, ["first", "last"]);
  assert.equal(state.hasMore, false);
  await act(() => state.reset());
  await act(() => requests[4].resolve({ items: [], next_cursor: null, has_more: false }));
  assert.deepEqual(state.items, []);
  assert.equal(state.hasMore, false);
  await act(() => state.reset());
  await act(() => component.unmount());
  assert.ok(requests[5].signal?.aborted);
});
