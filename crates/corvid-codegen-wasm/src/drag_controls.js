function bindDrag(region, handlers) {
  const output = region.querySelector(":scope > legend > output");
  const alert = region.querySelector(":scope > [role=alert]");
  const reduce = handlers[region.dataset.corvidDrag];
  const canvas = region.querySelector(":scope > canvas");
  const panel = region.querySelector(":scope > [data-corvid-panel]");
  const control = canvas ?? panel.querySelector(":scope > [role=separator]");
  const context = canvas?.getContext("2d");
  if (typeof reduce !== "function" || (canvas && !context)) {
    alert.textContent = "Could not initialize the drag action.";
    return;
  }
  let state = BigInt(output.value);
  let active = null;
  let last = null;
  let cursor = {x: 0, y: 0};
  const clamp = (n, min, max) => Math.max(min, Math.min(max, n));
  function point(e) {
    const rect = (canvas ?? panel).getBoundingClientRect();
    const x = e.clientX - rect.left, y = e.clientY - rect.top;
    if (!canvas) return {x: Math.round(x), y: Math.round(y)};
    return {x: clamp(Math.floor(x * canvas.width / rect.width), 0, canvas.width - 1),
            y: clamp(Math.floor(y * canvas.height / rect.height), 0, canvas.height - 1)};
  }
  function validPoint(p) {
    return p && Object.keys(p).length === 2 && Number.isSafeInteger(p.x) && Number.isSafeInteger(p.y) &&
      (!canvas || (p.x >= 0 && p.y >= 0 && p.x < canvas.width && p.y < canvas.height));
  }
  function invoke(stroke) {
    if (!region.isConnected || !stroke || Object.keys(stroke).length !== 2 ||
        !validPoint(stroke.start) || !validPoint(stroke.end)) throw Error("Expected a valid Stroke.");
    const {start, end} = stroke;
    let next = reduce(state, BigInt(start.x), BigInt(start.y), BigInt(end.x), BigInt(end.y));
    if (canvas?.hasAttribute("data-corvid-line")) {
      context.beginPath();
      context.moveTo(start.x + 0.5, start.y + 0.5);
      context.lineTo(end.x + 0.5, end.y + 0.5);
      if (start.x === end.x && start.y === end.y) context.fillRect(end.x, end.y, 1, 1);
      else context.stroke();
    } else if (panel) {
      const min = BigInt(control.getAttribute("aria-valuemin"));
      const max = BigInt(control.getAttribute("aria-valuemax"));
      next = next < min ? min : next > max ? max : next;
      panel.style.width = next + "px";
      control.setAttribute("aria-valuenow", next.toString());
    }
    state = next;
    output.value = state.toString();
    alert.textContent = "";
    return output.value;
  }
  function apply(start, end) {
    try { invoke({start, end}); cursor = end; }
    catch (error) { alert.textContent = error.message; }
  }
  control.addEventListener("pointerdown", e => {
    if (e.button !== 0 || active !== null) return;
    control.setPointerCapture(e.pointerId);
    active = e.pointerId;
    last = point(e);
    if (canvas) apply(last, last);
  });
  function move(e) {
    if (active !== e.pointerId) return;
    const end = point(e);
    if (last.x !== end.x || last.y !== end.y) apply(last, end);
    last = end;
  }
  control.addEventListener("pointermove", move);
  control.addEventListener("pointerup", e => { if (active === e.pointerId) { move(e); active = null; } });
  for (const type of ["pointercancel", "lostpointercapture"]) {
    control.addEventListener(type, e => { if (active === e.pointerId) active = null; });
  }
  control.addEventListener("keydown", e => {
    const keys = {ArrowRight:[1,0], ArrowLeft:[-1,0], ArrowUp:[0,-1], ArrowDown:[0,1]};
    const delta = Object.hasOwn(keys, e.key) ? keys[e.key] : null;
    if (!delta || (!canvas && delta[1]) || active !== null) return;
    e.preventDefault();
    const step = e.shiftKey ? 10 : 1;
    const end = {x: cursor.x + delta[0] * step, y: cursor.y + delta[1] * step};
    if (canvas) {
      end.x = clamp(end.x, 0, canvas.width - 1);
      end.y = clamp(end.y, 0, canvas.height - 1);
    }
    apply(cursor, end);
  });
  const owner = region.parentElement;
  if (owner.hasAttribute("data-corvid-drag-owner") && document.modelContext?.registerTool) {
    const pointSchema = {type:"object", required:["x","y"], additionalProperties:false,
      properties:{x:{type:"integer"}, y:{type:"integer"}}};
    if (canvas) {
      Object.assign(pointSchema.properties.x, {minimum:0, maximum:canvas.width - 1});
      Object.assign(pointSchema.properties.y, {minimum:0, maximum:canvas.height - 1});
    }
    Promise.resolve().then(() => document.modelContext.registerTool({
      name:owner.dataset.corvidTool, description:owner.dataset.corvidDescription,
      inputSchema:{type:"object", required:["start","end"], additionalProperties:false,
        properties:{start:pointSchema, end:pointSchema}},
      execute:invoke,
    })).catch(error => { alert.textContent = error.message; });
  }
}
