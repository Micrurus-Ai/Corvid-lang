import { instantiate } from "./target/wasm/viewport.js";

const viewport = await instantiate();
const tiles = [
  { name: "Lake", x: 24, y: 24 },
  { name: "Forest", x: 88, y: 40 },
];
let selected = 0;
let scale = 1;
let offsetX = 0;
let offsetY = 0;
const history = [];

function position(world, offset) {
  return Number(viewport.screen_position(BigInt(world), BigInt(scale), BigInt(offset)));
}

function visibleTiles() {
  return tiles.map(tile => ({
    ...tile, left: position(tile.x, offsetX), top: position(tile.y, offsetY), size: 32 * scale,
  }));
}

function publish() {
  self.postMessage({ tiles: visibleTiles(), selected, scale, offsetX, offsetY, canUndo: history.length > 0 });
}

self.onmessage = ({ data }) => {
  switch (data.action) {
    case "select":
      selected = data.index;
      break;
    case "pick":
      selected = visibleTiles().findLastIndex(tile =>
        viewport.contains_pixel(BigInt(data.x), BigInt(tile.left), BigInt(tile.size)) &&
        viewport.contains_pixel(BigInt(data.y), BigInt(tile.top), BigInt(tile.size)));
      break;
    case "pan":
      offsetX += data.x;
      offsetY += data.y;
      break;
    case "zoom":
      scale = Number(viewport.zoom_level(BigInt(scale), BigInt(data.change)));
      break;
    case "move":
      if (selected >= 0) {
        const tile = tiles[selected];
        history.push({ index: selected, x: tile.x, y: tile.y });
        tile.x += data.x;
        tile.y += data.y;
      }
      break;
    case "undo":
      if (history.length) {
        const edit = history.pop();
        selected = edit.index;
        tiles[selected].x = edit.x;
        tiles[selected].y = edit.y;
      }
      break;
  }
  publish();
};
publish();
