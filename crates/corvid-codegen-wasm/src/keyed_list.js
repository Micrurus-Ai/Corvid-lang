function bindKeyedList(region) {
  const list = region.querySelector(":scope > ol");
  const status = region.querySelector(":scope > [role=status]");
  const rows = new Map();
  for (const row of list.children) {
    const key = row.dataset.corvidKey;
    if (!key || rows.has(key)) {
      status.textContent = "Expected a unique key in " + list.getAttribute("aria-label") + ": " + (key ?? "(missing)");
      return;
    }
    rows.set(key, row);
  }
  let rowRange;
  function move(row, direction) {
    const up = direction === "up";
    const neighbor = up ? row.previousElementSibling : row.nextElementSibling;
    if (neighbor) {
      // Detach only the unfocused sibling, including when a pointer triggers the move.
      if (neighbor.contains(region.getRootNode().activeElement)) {
        list.insertBefore(row, up ? neighbor : neighbor.nextElementSibling);
      } else {
        list.insertBefore(neighbor, up ? row.nextElementSibling : row);
      }
    }
    // Locate the row without allocating an array of every sibling. Range offsets
    // include text and comments, so use element indices for hand-authored lists.
    let position;
    if (row.parentElement !== list) {
      position = 0;
    } else if (!row.previousElementSibling) {
      position = 1;
    } else if (!row.nextElementSibling) {
      position = list.children.length;
    } else if (list.childNodes.length === list.children.length) {
      rowRange ??= row.ownerDocument.createRange();
      rowRange.selectNode(row);
      position = rowRange.startOffset + 1;
    } else {
      position = Array.prototype.indexOf.call(list.children, row) + 1;
    }
    status.textContent = row.dataset.corvidKey + ", position " + position + " of " + rows.size;
  }
  const owner = region.closest("[data-reorder-owner]");
  if (owner && rows.size && document.modelContext?.registerTool && owner.querySelectorAll("[data-browser-rows]").length === 1) {
    Promise.resolve().then(() => document.modelContext.registerTool({
      name: owner.dataset.corvidTool, description: owner.dataset.corvidDescription,
      inputSchema: {type: "object", additionalProperties: false, required: ["key", "direction"],
        properties: {key: {type: "string", enum: Array.from(rows.keys())}, direction: {type: "string", enum: ["up", "down"]}}},
      execute(input) {
        if (!region.isConnected || !input || Object.keys(input).length !== 2 ||
            !rows.has(input.key) || !["up", "down"].includes(input.direction)) throw Error("Invalid row move.");
        move(rows.get(input.key), input.direction);
        return status.textContent;
      },
    })).catch(error => { status.textContent = error.message; });
  }
  for (const [key, row] of rows) {
    for (const button of row.querySelectorAll(":scope > button[data-corvid-move]")) {
      button.addEventListener("click", () => move(rows.get(key), button.dataset.corvidMove));
      button.hidden = false;
    }
    row.addEventListener("keydown", event => {
      if (event.target.closest("[data-corvid-list], [data-browser-rows]") !== region || event.isComposing ||
          !event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        move(rows.get(key), event.key === "ArrowUp" ? "up" : "down");
      }
    });
  }
}
