import "./camera.js";

const canvas = document.querySelector("canvas");
const gl = canvas.getContext("webgl", { antialias: false });
const graphics = document.querySelector("#graphics");
const selection = document.querySelector("#selection");
const positions = document.querySelector("#positions");
const worker = new Worker("./scene-worker.js", { type: "module" });
let scene;
let scheduled = false;

function paint() {
  scheduled = false;
  if (!scene || !gl || gl.isContextLost()) return;
  const bounds = canvas.getBoundingClientRect();
  const width = Math.round(bounds.width * devicePixelRatio);
  const height = Math.round(bounds.height * devicePixelRatio);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  gl.disable(gl.SCISSOR_TEST);
  gl.clearColor(0.94, 0.96, 0.95, 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.enable(gl.SCISSOR_TEST);
  function rectangle(left, top, size, color) {
    const x = Math.round(left * width / bounds.width);
    const right = Math.round((left + size) * width / bounds.width);
    const y = Math.round(top * height / bounds.height);
    const bottom = Math.round((top + size) * height / bounds.height);
    gl.scissor(x, height - bottom, right - x, bottom - y);
    gl.clearColor(...color, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }
  scene.tiles.forEach(({ left, top, size }, index) => {
    if (index === scene.selected) rectangle(left - 2, top - 2, size + 4, [0.1, 0.1, 0.1]);
    rectangle(left, top, size, index === 0 ? [0.1, 0.45, 0.8] : [0.12, 0.55, 0.25]);
  });
}

function schedulePaint() {
  if (!scheduled) {
    scheduled = true;
    requestAnimationFrame(paint);
  }
}

function graphicsReady() {
  graphics.textContent = gl ? "Graphics ready." : "Graphics unavailable. Use the scene controls and tile positions.";
  schedulePaint();
}
canvas.addEventListener("webglcontextlost", event => {
  event.preventDefault();
  graphics.textContent = "Graphics lost. Use the scene controls and tile positions.";
});
canvas.addEventListener("webglcontextrestored", graphicsReady);
graphicsReady();
new ResizeObserver(schedulePaint).observe(canvas);
function watchDensity() {
  matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener("change", () => {
    schedulePaint();
    watchDensity();
  }, { once: true });
}
watchDensity();

worker.onmessage = ({ data }) => {
  scene = data;
  if (selection.options.length === 1) {
    scene.tiles.forEach((tile, index) => {
      selection.add(new Option(tile.name, String(index)));
      positions.append(document.createElement("li"));
    });
  }
  selection.value = String(scene.selected);
  scene.tiles.forEach((tile, index) => {
    positions.children[index].textContent = `${tile.name} at (${tile.x}, ${tile.y})`;
  });
  document.querySelector("#description").textContent =
    `Zoom ${scene.scale}×. Pan (${scene.offsetX}, ${scene.offsetY}). Selected: ${scene.tiles[scene.selected]?.name ?? "None"}.`;
  document.querySelector("#controls").disabled = false;
  document.querySelector("#undo").disabled = !scene.canUndo;
  document.querySelector("#smaller").disabled = scene.scale === 1;
  document.querySelector("#larger").disabled = scene.scale === 4;
  document.querySelectorAll("#move button").forEach(button => { button.disabled = scene.selected < 0; });
  schedulePaint();
};
worker.onerror = () => {
  scene = null;
  document.querySelector("#controls").disabled = true;
  graphics.textContent = "Scene worker unavailable. Check the build and reload.";
};
selection.onchange = () => worker.postMessage({ action: "select", index: Number(selection.value) });
for (const action of ["pan", "move"]) {
  document.querySelectorAll(`#${action} button`).forEach(button => {
    button.onclick = () => worker.postMessage({
      action, x: Number(button.dataset.x ?? 0), y: Number(button.dataset.y ?? 0),
    });
  });
}
document.querySelector("#undo").onclick = () => worker.postMessage({ action: "undo" });
document.querySelector("#smaller").onclick = () => worker.postMessage({ action: "zoom", change: -1 });
document.querySelector("#larger").onclick = () => worker.postMessage({ action: "zoom", change: 1 });

let drag;
canvas.onpointerdown = event => {
  if (!scene || drag || event.button !== 0) return;
  canvas.setPointerCapture(event.pointerId);
  drag = { id: event.pointerId, x: Math.round(event.clientX), y: Math.round(event.clientY), moved: false };
};
function panPointer(event) {
  if (drag?.id !== event.pointerId) return;
  const x = Math.round(event.clientX);
  const y = Math.round(event.clientY);
  const dx = x - drag.x;
  const dy = y - drag.y;
  if (!drag.moved && Math.max(Math.abs(dx), Math.abs(dy)) < 3) return;
  if (dx || dy) worker.postMessage({ action: "pan", x: dx, y: dy });
  Object.assign(drag, { x, y, moved: true });
}
canvas.onpointermove = panPointer;
canvas.onpointerup = event => {
  if (drag?.id !== event.pointerId) return;
  panPointer(event);
  if (!drag.moved) {
    const bounds = canvas.getBoundingClientRect();
    worker.postMessage({ action: "pick", x: Math.floor(event.clientX - bounds.left),
      y: Math.floor(event.clientY - bounds.top) });
  }
  drag = null;
};
canvas.onlostpointercapture = canvas.onpointercancel = event => {
  if (drag?.id === event.pointerId) drag = null;
};
