import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { gzipSync } from "node:zlib";
import test from "node:test";
import { readNote } from "./target/wasm/read_note.js";
import { instantiate } from "./target/wasm/events.js";

test("read a nullable gzip file and preserve UTF-8 text", async () => {
  const quote = text => "[" + text + "]";
  assert.equal(await readNote(null, quote), "[]");
  assert.throws(() => readNote("note.gz", quote), /Choose a gzip file/);
  const note = "Café\nShopping list";
  assert.equal(await readNote(new File([gzipSync(note)], "note.gz"), quote), quote(note));
  await assert.rejects(readNote(new File(["invalid archive"], "broken.gz"), quote));
  await assert.rejects(readNote(new File([gzipSync(Buffer.from([0xff]))], "binary.gz"), quote));
});

test("pass decoded text to the compiled Corvid callback", async context => {
  context.mock.method(globalThis, "fetch", async url => new Response(await readFile(url), {
    headers: { "Content-Type": "application/wasm" },
  }));
  const handlers = await instantiate();
  const note = "First line\n雪";
  const file = new File([gzipSync(note)], "note.gz");
  assert.equal(await readNote(file, handlers.imported_text), note);
});
