// Drives the running app through WebView2's debug port for end-to-end checks.
// Usage: node tools/cdp.mjs eval "<js>" | shot <file.png> [width height]
const [cmd, a, w, h] = process.argv.slice(2);
const list = await (await fetch("http://127.0.0.1:9222/json")).json();
const t = list.find((x) => x.type === "page");
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const wait = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (wait.has(d.id)) { wait.get(d.id)(d); wait.delete(d.id); } };
const send = (method, params = {}) => new Promise((r) => { const i = ++id; wait.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
if (cmd === "eval") {
  const r = await send("Runtime.evaluate", { expression: a, awaitPromise: true, returnByValue: true });
  console.log(JSON.stringify(r.result?.result?.value ?? r.result?.exceptionDetails?.exception?.description ?? r.result));
} else if (cmd === "shot") {
  if (w) await send("Emulation.setDeviceMetricsOverride", { width: +w, height: +h, deviceScaleFactor: 1, mobile: false });
  await new Promise((r) => setTimeout(r, 500));
  const r = await send("Page.captureScreenshot", { format: "png" });
  (await import("node:fs")).writeFileSync(a, Buffer.from(r.result.data, "base64"));
  if (w) await send("Emulation.clearDeviceMetricsOverride");
  console.log("saved", a);
}
ws.close();
