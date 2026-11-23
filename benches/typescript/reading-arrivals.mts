/** Send loopback edits on a fixed clock, even while earlier requests are outstanding. */
import http from "node:http";
import {performance} from "node:perf_hooks";

const [origin, label, secondsText, writeRateText, readRateText] = process.argv.slice(2);
const seconds = Number(secondsText);
const agent = new http.Agent({keepAlive: true});
type Arrival = {method: "GET" | "POST"; i: number; due: number};
type Sample = [Arrival["method"], number, number, number, number, string | null];

const schedule: Arrival[] = [];
for (const [method, rate] of [["POST", Number(writeRateText)], ["GET", Number(readRateText)]] as const) {
  for (let i = 0; i < seconds * rate; i++) {
    // Offset readers half a read interval from the start of the write stream.
    schedule.push({method, i, due: 1000 * (i + (method === "GET" ? 0.5 : 0)) / rate});
  }
}
schedule.sort((a, b) => a.due - b.due);
const samples: Sample[] = [];
const start = performance.now();
const elapsed = () => Math.round((performance.now() - start) * 100) / 100;
console.log("started");

function request({method, i, due}: Arrival): Promise<void> {
  const write = method === "POST";
  const body = write ? new URLSearchParams({shelf: String(1 + i % 2), title: label + "-" + i}).toString() : "";
  const sent = elapsed();
  return new Promise(resolve => {
    const address = new URL(write ? "/save" : "/edit?shelf=3", origin);
    const req = http.request(address, {
      agent, method, headers: write ? {"Content-Type": "application/x-www-form-urlencoded",
        "Content-Length": Buffer.byteLength(body)} : {},
    }, res => {
      let html = "";
      res.setEncoding("utf8");
      res.on("data", chunk => { html += chunk; });
      res.on("error", err => finish(err.message));
      res.on("end", () => {
        const confirmed = html.includes(write ? "<p>Saved</p>" : "Save to shelf");
        finish(res.statusCode === 200 && confirmed ? null : "Invalid response " + res.statusCode);
      });
    });
    let done = false;
    function finish(error: string | null) {
      if (done) return;
      done = true;
      clearTimeout(deadline);
      samples.push([method, i, Math.round(due * 100) / 100, sent, elapsed(), error]);
      resolve();
    }
    const deadline = setTimeout(() => req.destroy(new Error("30-second deadline")), 30000);
    req.on("error", err => finish(err.message));
    req.end(body);
  });
}

const pending: Promise<void>[] = [];
for (const item of schedule) {
  const wait = item.due - elapsed();
  if (wait > 0) await new Promise(resolve => setTimeout(resolve, wait));
  pending.push(request(item));
}
await Promise.all(pending);
agent.destroy();
console.log(JSON.stringify(samples));
