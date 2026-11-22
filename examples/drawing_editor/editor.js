const canvas = document.querySelector("canvas");
const status = document.querySelector('[role="status"]');
const context = canvas.getContext("2d");

function enableDrawing(worker) {
  status.textContent = "Drag to draw.";
  let pointer = null;
  let previous;
  function point(event) {
    const rect = canvas.getBoundingClientRect();
    const x = Math.floor((event.clientX - rect.left) * canvas.width / rect.width);
    const y = Math.floor((event.clientY - rect.top) * canvas.height / rect.height);
    return [Math.max(0, Math.min(canvas.width - 1, x)), Math.max(0, Math.min(canvas.height - 1, y))];
  }
  function extend(event) {
    const next = point(event);
    worker.postMessage([...previous, ...next]);
    previous = next;
  }
  canvas.onpointerdown = event => {
    if (pointer !== null || event.button !== 0) return;
    pointer = event.pointerId;
    canvas.setPointerCapture(pointer);
    previous = point(event);
    extend(event);
  };
  canvas.onpointermove = event => {
    if (event.pointerId === pointer) extend(event);
  };
  canvas.onpointerup = event => {
    if (event.pointerId === pointer) {
      extend(event);
      pointer = null;
    }
  };
  canvas.onlostpointercapture = canvas.onpointercancel = event => {
    if (event.pointerId === pointer) pointer = null;
  };
}

if (typeof Worker === "undefined" || !context) {
  status.textContent = "Drawing requires a browser with canvas and worker support.";
} else {
  const worker = new Worker("./stroke-worker.js", { type: "module" });
  worker.onmessage = ({ data }) => {
    if (data === "ready") {
      enableDrawing(worker);
      return;
    }
    for (let i = 0; i < data.length; i += 2) {
      context.fillRect(data[i], data[i + 1], 1, 1);
    }
  };
  worker.onerror = () => {
    canvas.onpointerdown = canvas.onpointermove = canvas.onpointerup = null;
    worker.terminate();
    status.textContent = "The drawing worker could not run. Check the WebAssembly build and reload.";
  };
}
