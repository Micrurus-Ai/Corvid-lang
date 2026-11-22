import { instantiate } from "./target/wasm/stroke.js";
import { rasterize } from "./raster.mjs";

const stroke = await instantiate();
self.onmessage = ({ data }) => {
  const cells = rasterize(stroke, data);
  self.postMessage(cells, [cells.buffer]);
};
self.postMessage("ready");
