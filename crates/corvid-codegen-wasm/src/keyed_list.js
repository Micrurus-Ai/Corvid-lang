function bindKeyedList(region, mountRecord) {
  const list = region.querySelector(":scope > ol");
  const status = region.querySelector(":scope > [role=status]");
  if (!list || !status) {
    const message = "std/ui.keyed_list: expected an ordered list and a status element.";
    if (status) status.textContent = message;
    else console.error(message, region);
    return;
  }
  const rows = new Map();
  for (const row of list.children) {
    const key = row.dataset.corvidKey;
    if (!key || rows.has(key)) {
      status.textContent = "Expected a unique key in " + list.getAttribute("aria-label") + ": " + (key ?? "(missing)");
      return;
    }
    rows.set(key, row);
  }
  const editor = mountRecord ? region.closest("[data-list-editor]") : null;
  const add = editor?.querySelector(":scope > [data-add-row]");
  let template;
  if (editor) {
    const templates = editor.querySelector(":scope > template")?.content.querySelectorAll("[data-browser-rows] > ol > li");
    if (!add || templates?.length !== 1 || editor.querySelectorAll("[data-browser-rows]").length !== 1) {
      status.textContent = "A list editor needs one list and one new-row template.";
      return;
    }
    template = templates[0];
  }
  let rowRange;
  function move(row, direction) {
    if (!row || row.parentElement !== list || list.parentElement !== region) return false;
    const button = row.querySelector(':scope > button[data-corvid-move="' + direction + '"]');
    if (!button || button.matches(":disabled")) return false;
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
    if (!row.previousElementSibling) {
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
    return true;
  }
  const owner = region.closest("[data-reorder-owner]");
  if (owner && (rows.size || editor) && document.modelContext?.registerTool && owner.querySelectorAll("[data-browser-rows]").length === 1) {
    const toolName = owner.dataset.corvidTool;
    const schema = {type: "object", additionalProperties: false,
      properties: {key: editor ? {type: "string"} : {type: "string", enum: Array.from(rows.keys())},
        direction: {type: "string", enum: ["up", "down"]}}};
    if (editor) {
      schema.oneOf = [{maxProperties: 0}, {required: ["key", "direction"]}];
      schema.description = "Send {} to read current row keys in order, or supply a key and direction to move.";
    } else schema.required = ["key", "direction"];
    Promise.resolve().then(() => document.modelContext.registerTool({
      name: toolName, description: owner.dataset.corvidDescription,
      inputSchema: schema,
      execute(input) {
        if (!region.isConnected || region.closest("[data-reorder-owner]") !== owner ||
            owner.dataset.corvidTool !== toolName || list.parentElement !== region) {
          throw Error("List action is unavailable.");
        }
        if (!input || typeof input !== "object" || Array.isArray(input)) {
          throw Error("Invalid row move.");
        }
        const count = Object.keys(input).length;
        if (editor && count === 0) return Array.from(list.children, row => row.dataset.corvidKey);
        if (count !== 2 || !rows.has(input.key) || !["up", "down"].includes(input.direction)) {
          throw Error("Invalid row move.");
        }
        if (!move(rows.get(input.key), input.direction)) throw Error("Row move is unavailable.");
        return status.textContent;
      },
    })).catch(error => { status.textContent = error.message; });
  }
  function preserveOtherInput(button, removedRow) {
    button.addEventListener("mousedown", event => {
      const active = region.getRootNode().activeElement;
      if (event.button === 0 && active?.matches("input,textarea,select") &&
          list.contains(active) && !removedRow?.contains(active)) event.preventDefault();
    });
  }

  function attachRow(key, row) {
    if (editor) {
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = editor.dataset.removeLabel;
      remove.setAttribute("aria-label", remove.textContent + " " + key);
      preserveOtherInput(remove, row);
      remove.addEventListener("click", () => {
        const hadFocus = row.contains(region.getRootNode().activeElement);
        const neighbor = row.nextElementSibling ?? row.previousElementSibling;
        rows.delete(key);
        row.remove();
        if (hadFocus) (neighbor?.querySelector("[data-remove-row]") ?? add).focus();
        status.textContent = "Removed " + key;
      });
      remove.dataset.removeRow = "";
      row.append(remove);
    }
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
  for (const [key, row] of rows) attachRow(key, row);
  if (!editor) return;

  let serial = 0;
  preserveOtherInput(add);
  add.addEventListener("click", () => {
    try {
      let key;
      do { key = template.dataset.corvidKey + "-" + (++serial); } while (rows.has(key));
      const row = template.cloneNode(true);
      const holder = row.querySelector(":scope > [data-row-key-field]");
      const record = holder.querySelector(":scope > [data-local-selection]");
      const keyNode = Array.from(record.querySelectorAll(":scope > [data-record-field]"))
        .find(node => JSON.parse(node.dataset.recordField).name === holder.dataset.rowKeyField);
      if (!keyNode || !template.dataset.corvidKey) throw Error("The new row needs a nonempty string key.");
      const descriptor = JSON.parse(keyNode.dataset.recordField);
      descriptor.value = key;
      keyNode.dataset.recordField = JSON.stringify(descriptor);
      row.dataset.corvidKey = key;
      for (const button of row.querySelectorAll(":scope > [data-corvid-move]")) {
        button.textContent = "Move " + key + (button.dataset.corvidMove === "up" ? " earlier" : " later");
      }
      mountRecord(record);
      attachRow(key, row);
      rows.set(key, row);
      list.append(row);
      status.textContent = "Added " + key;
    } catch (error) { status.textContent = error.message; }
  });
  add.hidden = false;
}
