import { Gunzip, type FlateStreamHandler } from "fflate";
import type { CorvidFieldReader } from "./target/wasm/events.js";

async function decompressNote(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let note = "";
  const appendText: FlateStreamHandler = (chunk, final) => {
    note += decoder.decode(chunk, { stream: !final });
  };
  const reader = new Gunzip(appendText);
  reader.push(bytes, true);
  return note;
}

export const readNote: CorvidFieldReader = (value, importedText) => {
  if (value === null) return importedText("");
  if (typeof value === "string") throw TypeError("Choose a gzip file.");
  return decompressNote(value).then(importedText);
};
