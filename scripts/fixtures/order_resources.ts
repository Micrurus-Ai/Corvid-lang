import { CorvidClient, CorvidError, type Page } from "./client/index.js";
import type {
  ImportReceipt, NewOrder, OrderCursor, OrderEvent, OrderFilter, OrderPage, OrderReceipt, OrderSummary,
} from "./generated/types.js";

function check(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

// Inject only the interrupted response. The successful stream uses the live orders route.
async function checkInterruptedActivity(baseUrl: string): Promise<void> {
  const encoder = new TextEncoder();
  const partial = 'data: {"id":"order-1","status":"picked"}\r\n';
  for (const ending of ["", "event: error\ndata: interrupted\n\n"]) {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode(partial + "\r"));
        controller.enqueue(encoder.encode("\n" + ending));
        controller.close();
      },
    });
    const client = new CorvidClient({ baseUrl, fetch: async () => new Response(body) });
    const activity = client.routeStream<OrderEvent>("/orders/activity");
    const first = await activity.next();
    check(!first.done && first.value.status === "picked", "Missing partial order activity");
    try {
      await activity.next();
      throw new Error("Interrupted stream reported success");
    } catch (error) {
      if (!(error instanceof CorvidError)) throw error;
    }
  }
}

export async function checkOrders(baseUrl: string): Promise<void> {
  const client = new CorvidClient({ baseUrl });
  const filter: OrderFilter = { status: "awaiting pickup", limit: 1 };
  const query = new URLSearchParams({ status: filter.status, limit: String(filter.limit) });
  const results = await client.request<OrderPage>("GET", "/orders?" + query);
  check(results.orders[0]?.status === filter.status, "Search returned the wrong status");

  const order: NewOrder = { item: "notebook", quantity: 2 };
  const receipt = await client.request<OrderReceipt>("POST", "/orders", order);
  check(receipt.accepted && receipt.id === "order-new", "Order was not accepted");

  // The orders fixture echoes its cursor. Limit the walk to two requests.
  const cursor: OrderCursor = { cursor: "start", limit: 2 };
  for (let count = 0; count < 2; count++) {
    const query = new URLSearchParams({ cursor: cursor.cursor, limit: String(cursor.limit) });
    const page = await client.request<Page<OrderSummary>>("GET", "/orders/page?" + query);
    check(page.items.length === 2 && page.items[1].status === "shipped", "Incorrect order page");
    if (!page.has_more || page.next_cursor === null) throw new Error("Missing continuation");
    cursor.cursor = page.next_cursor;
  }

  const statuses = [];
  for await (const update of client.routeStream<OrderEvent>("/orders/activity")) {
    statuses.push(update.status);
  }
  check(statuses.join(",") === "placed,picked,shipped", "Incomplete order activity");
  await checkInterruptedActivity(baseUrl);

  const cancelled = AbortSignal.abort();
  try {
    await client.request<OrderPage>("GET", "/orders?" + query, undefined, cancelled);
    throw new Error("Cancelled search completed");
  } catch (error) {
    if (error !== cancelled.reason) throw error;
  }
}

export async function checkOrderUpload(baseUrl: string): Promise<void> {
  const client = new CorvidClient({ baseUrl });
  const path = "/orders/import";
  const upload = (file: File, signal?: AbortSignal) =>
    client.upload<ImportReceipt>(path, file, (loaded, total) => {
      progress.push({ loaded, total });
    }, signal);
  const progress: Array<{ loaded: number; total?: number }> = [];
  try {
    await upload(new File(["unrecognized"], "orders.pdf", { type: "application/pdf" }));
    throw new Error("Wrong file type accepted");
  } catch (error) {
    if (!(error instanceof CorvidError && error.status === 400)) throw error;
  }
  progress.length = 0;
  const file = new File(["item,quantity\nnotebook,2\n"], "orders.csv", { type: "text/csv" });
  const receipt = await upload(file);
  check(receipt.filename === file.name && receipt.bytes === file.size, "Wrong import receipt");
  const last = progress.at(-1);
  check(!!last && last.loaded >= file.size && last.loaded === last.total, "Missing transfer progress");

  const controller = new AbortController();
  const pending = upload(file, controller.signal);
  controller.abort();
  try {
    await pending;
    throw new Error("Cancelled upload completed");
  } catch (error) {
    if (error !== controller.signal.reason) throw error;
  }
}
