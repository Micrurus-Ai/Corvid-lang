import assert from "node:assert/strict";
import { test } from "node:test";
import { CorvidClient, CorvidError } from "../target/index.js";

// Drive the browser-facing WebSocket boundary without running an endpoint.
function setup(t) {
  const sockets = [];
  class Socket {
    static OPEN = 1;
    sent = [];
    constructor(url, protocol) {
      assert.equal(url.href, "wss://example.test/.well-known/corvid/live");
      assert.equal(protocol, "corvid.live.v1");
      sockets.push(this);
    }
    open() { this.readyState = 1; this.onopen(); }
    send(text) { this.sent.push(JSON.parse(text)); }
    deliver(id, data) { this.onmessage({ data: JSON.stringify({ id, ...data }) }); }
    close(code) { this.readyState = 3; this.onclose({ code }); }
  }
  const previous = globalThis.WebSocket;
  globalThis.WebSocket = Socket;
  t.after(() => { globalThis.WebSocket = previous; });
  const client = new CorvidClient({ baseUrl: "https://example.test" });
  function subscribe(path) {
    const updates = [], errors = [];
    const stop = client.subscribe(path, ["board", "card"], {
      rows: rows => updates.push(rows),
      error: error => errors.push(error),
    });
    t.after(stop);
    return { stop, updates, errors };
  }
  return { sockets, subscribe };
}

const card = (board, card, title, revision = "9007199254740992") =>
  ({ row: { board, card, title }, revision });

test("reconnect reconciles by composite key and exact revision, then accepts ordered changes", t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { sockets, subscribe } = setup(t);
  const { updates } = subscribe("/cards");
  const socket = sockets[0];
  socket.open();
  const [{ id }] = socket.sent;
  const a = card("a", 1, "keep");
  const b = card("b", 1, "edit");
  const c = card("c", 1, "remove");
  socket.deliver(id, { type: "snapshot", rows: [a, b, c] });
  socket.close(1006);
  t.mock.timers.runAll();
  const next = sockets[1];
  next.open();
  assert.deepEqual(next.sent, [{ type: "subscribe", id, path: "/cards" }]);
  socket.deliver(id, { type: "snapshot", rows: [] });
  const edited = card("b", 1, "missed edit", "9007199254740993");
  next.deliver(id, { type: "snapshot", rows: [edited, a] });
  assert.deepEqual(updates[1], [edited.row, a.row]);
  assert.equal(updates[0][0], updates[1][1]);
  assert.notEqual(updates[0][1], updates[1][0]);
  next.deliver(id, { type: "change", kind: "added", index: 1, ...c });
  next.deliver(id, { type: "change", kind: "updated", index: 2, ...card("b", 1, "later", "9007199254740994") });
  next.deliver(id, { type: "change", kind: "removed", ...c });
  assert.deepEqual(updates.at(-1).map(row => row.title), ["keep", "later"]);
  assert.equal(updates[0][0], updates.at(-1)[0]);
});

test("forbidden retires one id and unsubscribe releases the last subscriber once", t => {
  const { sockets, subscribe } = setup(t);
  const first = subscribe("/cards");
  const socket = sockets[0];
  socket.open();
  const second = subscribe("/other/cards");
  assert.equal(sockets.length, 1);
  const [a, b] = socket.sent.map(frame => frame.id);
  assert.notEqual(a, b);
  socket.deliver(a, { type: "error", code: "forbidden" });
  assert.ok(first.errors[0] instanceof CorvidError);
  assert.equal(first.errors[0].status, 403);
  socket.deliver(a, { type: "snapshot", rows: [] });
  assert.deepEqual(first.updates, []);
  socket.deliver(b, { type: "snapshot", rows: [] });
  assert.deepEqual(second.updates, [[]]);
  first.stop();
  second.stop();
  second.stop();
  assert.deepEqual(socket.sent.slice(2), [{ type: "unsubscribe", id: b }]);
  assert.equal(socket.readyState, 3);
});

test("invalid replacement snapshot never publishes partial or collapsed rows", async t => {
  const { sockets, subscribe } = setup(t);
  const sub = subscribe("/cards");
  const socket = sockets[0];
  socket.open();
  const [{ id }] = socket.sent;
  const a = card("board", 1, "first");
  socket.deliver(id, { type: "snapshot", rows: [a] });
  socket.deliver(id, { type: "snapshot", rows: [a, a] });
  await Promise.resolve();
  assert.deepEqual(sub.updates, [[a.row]]);
  assert.match(sub.errors[0].message, /duplicate/i);
});

test("session invalidation ends every subscription without retrying", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { sockets, subscribe } = setup(t);
  const a = subscribe("/cards"), b = subscribe("/other/cards");
  sockets[0].open();
  sockets[0].close(1008);
  await Promise.resolve();
  for (const sub of [a, b]) assert.equal(sub.errors[0].body.code, "invalid_session");
  t.mock.timers.runAll();
  assert.equal(sockets.length, 1);
});

test("unmount while disconnected cancels resubscription", t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { sockets, subscribe } = setup(t);
  const sub = subscribe("/cards");
  sockets[0].open();
  sockets[0].close(1006);
  sub.stop();
  t.mock.timers.runAll();
  assert.equal(sockets.length, 1);
});
