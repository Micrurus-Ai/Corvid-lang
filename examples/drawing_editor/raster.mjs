// Return integer cell coordinates in a buffer the worker can transfer.
export function rasterize(stroke, [x0, y0, x1, y1]) {
  const count = stroke.steps(BigInt(x1 - x0), BigInt(y1 - y0));
  const cells = new Uint16Array((Number(count) + 1) * 2);
  let offset = 0;
  for (let step = 0n; step <= count; step++) {
    cells[offset++] = Number(stroke.coordinate(BigInt(x0), BigInt(x1), step, count));
    cells[offset++] = Number(stroke.coordinate(BigInt(y0), BigInt(y1), step, count));
  }
  return cells;
}
