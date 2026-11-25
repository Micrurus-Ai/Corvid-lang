import { instantiate } from "./target/wasm/inventory.js";

const inventory = await instantiate();
const viewport = document.querySelector("#viewport");
const receive = document.querySelector("#receive");
const status = document.querySelector("#status");
const heading = document.querySelector("h1");
const height = 32;
const initialCount = 10000;

function openCollection(table) {
    const saved = history.state;
    let count = saved?.count ?? initialCount;
    let active = saved?.active ?? 0;
    const rows = document.createElement(table ? "tbody" : "ul");
    rows.id = "rows";
    const collection = table ? document.createElement("table") : rows;
    collection.setAttribute("aria-label", "Storage bins");
    if (table) collection.append(rows);
    viewport.append(collection);
    const mounted = new Map();

    function createRow(index) {
        const row = document.createElement(table ? "tr" : "li");
        row.dataset.row = index;
        row.style.height = height + "px";
        row.style.top = index * height + "px";
        const link = document.createElement("a");
        link.textContent = "Bin " + (index + 1);
        link.href = "?view=" + (table ? "table" : "list") + "&bin=" + (index + 1);
        const stock = inventory.stock(BigInt(index + 1)) + " parts";
        if (table) {
            row.setAttribute("aria-rowindex", index + 1);
            const heading = document.createElement("th");
            heading.scope = "row";
            heading.append(link);
            row.append(heading);
            row.insertCell().textContent = stock;
        } else {
            row.setAttribute("aria-posinset", index + 1);
            row.append(link, document.createTextNode(stock));
        }
        return row;
    }

    function render(restoreFocus = false) {
        rows.style.height = count * height + "px";
        if (table) collection.setAttribute("aria-rowcount", count);
        const scroll = BigInt(Math.max(0, Math.floor(viewport.scrollTop)));
        const first = Number(inventory.first_row(scroll, BigInt(height)));
        const end = Number(inventory.end_row(scroll, BigInt(viewport.clientHeight), BigInt(height), BigInt(count)));
        const focused = restoreFocus === true || rows.contains(document.activeElement);
        if (!focused && (active < first || active >= end)) active = first;
        for (const [index, row] of mounted) {
            if ((index < first || index >= end) && !(focused && index === active)) {
                row.remove();
                mounted.delete(index);
            }
        }
        const indices = Array.from({length: end - first}, (_, offset) => first + offset);
        if (focused && (active < first || active >= end)) indices.push(active);
        for (const index of indices) {
            if (mounted.has(index)) continue;
            const row = createRow(index);
            const next = Array.from(rows.children).find(child => Number(child.dataset.row) > index);
            rows.insertBefore(row, next ?? null);
            mounted.set(index, row);
        }
        for (const [index, row] of mounted) {
            row.querySelector("a").tabIndex = index === active ? 0 : -1;
            if (!table) row.setAttribute("aria-setsize", count);
        }
    }

    rows.style.height = count * height + "px";
    viewport.scrollTop = saved?.scroll ?? 0;
    render(saved?.focused);
    if (saved?.focused) mounted.get(active)?.querySelector("a").focus({preventScroll: true});
    viewport.addEventListener("scroll", render);
    rows.addEventListener("focusin", event => {
        active = Number(event.target.closest("[data-row]").dataset.row);
        render();
    });
    rows.addEventListener("keydown", event => {
        const step = Math.floor(viewport.clientHeight / height);
        const destinations = {ArrowDown: active + 1, ArrowUp: active - 1,
            PageDown: active + step, PageUp: active - step, Home: 0, End: count - 1};
        if (!Object.hasOwn(destinations, event.key)) return;
        event.preventDefault();
        active = Math.max(0, Math.min(count - 1, destinations[event.key]));
        const top = active * height;
        if (top < viewport.scrollTop) viewport.scrollTop = top;
        if (top + height > viewport.scrollTop + viewport.clientHeight) {
            viewport.scrollTop = top + height - viewport.clientHeight;
        }
        render();
        mounted.get(active).querySelector("a").focus({preventScroll: true});
    });
    function remember() {
        history.replaceState({scroll: viewport.scrollTop, active, count,
            focused: rows.contains(document.activeElement)}, "");
    }
    rows.addEventListener("click", event => {
        const link = event.target.closest("a");
        if (!link || event.defaultPrevented || event.button !== 0 || event.metaKey ||
                event.ctrlKey || event.shiftKey || event.altKey) return;
        event.preventDefault();
        remember();
        history.pushState(null, "", link.href);
        renderLocation();
        heading.focus();
    });
    let shipment;
    function stopShipment() {
        shipment?.close();
        receive.disabled = count === initialCount + 32;
    }
    window.addEventListener("pagehide", stopShipment);
    receive.disabled = count === initialCount + 32;
    receive.onclick = () => {
        receive.disabled = true;
        shipment = new EventSource("shipment");
        shipment.onmessage = event => {
            const sequence = Number(event.data);
            if (!Number.isInteger(sequence) || sequence < 1 || sequence > 32 ||
                    sequence > count - initialCount + 1) {
                shipment.onerror();
                return;
            }
            count = Math.max(count, initialCount + sequence);
            render();
            status.value = count + " bins";
            if (count === initialCount + 32) shipment.close();
        };
        shipment.onerror = () => {
            shipment.close();
            receive.disabled = false;
            status.value = "Shipment interrupted. Receive again to resume.";
        };
    };
    return () => {
        stopShipment();
        viewport.removeEventListener("scroll", render);
        window.removeEventListener("pagehide", stopShipment);
        receive.onclick = null;
    };
}

let closeCollection;
function renderLocation() {
    closeCollection?.();
    closeCollection = undefined;
    viewport.replaceChildren();
    const query = new URLSearchParams(location.search);
    const bin = Number(query.get("bin"));
    const detail = Number.isInteger(bin) && bin >= 1 && bin <= initialCount + 32;
    receive.hidden = detail;
    heading.textContent = detail ? "Bin " + bin : "Parts inventory";
    status.value = "";
    if (detail) {
        viewport.textContent = inventory.stock(BigInt(bin)) + " parts";
    } else {
        closeCollection = openCollection(query.get("view") === "table");
    }
}
heading.tabIndex = -1;
window.addEventListener("popstate", renderLocation);
renderLocation();
performance.mark("inventory-ready");
