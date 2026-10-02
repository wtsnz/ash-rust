// How the command center holds up in the browser as the fleet grows.
//
// Opens the page in Chrome, lets it settle, then measures frame times, main-thread long
// tasks, the JS heap, and what arrives over the live WebSocket. Start the API with a
// fleet first (`FLEET=5000 cargo run -p cybercab --release`) and serve the front end
// against it, then:
//
//   node scripts/bench-browser.mjs --url http://localhost:4321 [--wall] [--seconds 20]
//
// Set CHROME to the browser binary if it isn't in the default macOS location.

import puppeteer from "puppeteer-core";

const flag = (name, fallback) => {
  const at = process.argv.indexOf(`--${name}`);
  if (at === -1) return fallback;
  const next = process.argv[at + 1];
  return next === undefined || next.startsWith("--") ? true : next;
};

const url = new URL(flag("url", "http://localhost:4321"));
if (flag("wall", false)) url.searchParams.set("mode", "wall");
const seconds = Number(flag("seconds", 20));
const warmup = Number(flag("warmup", 10));
const [width, height] = flag("wall", false) ? [1920, 1080] : [1600, 900];

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--ignore-gpu-blocklist", "--enable-gpu-rasterization"],
});
const page = await browser.newPage();
await page.setViewport({ width, height, deviceScaleFactor: 1 });

// Count what the live connection delivers, before the page opens it.
await page.evaluateOnNewDocument(() => {
  window.__ws = { messages: 0, bytes: 0, missed: 0 };
  const Native = window.WebSocket;
  window.WebSocket = class extends Native {
    constructor(...args) {
      super(...args);
      this.addEventListener("message", (event) => {
        window.__ws.messages += 1;
        window.__ws.bytes += typeof event.data === "string" ? event.data.length : event.data.size ?? 0;
        // A subscription that fell behind the server, which re-reads to catch up.
        if (typeof event.data === "string" && event.data.includes("MISSED_EVENTS")) window.__ws.missed += 1;
      });
    }
  };
});

await page.goto(url.toString(), { waitUntil: "networkidle2", timeout: 120_000 });
await new Promise((resolve) => setTimeout(resolve, warmup * 1000));

const result = await page.evaluate(async (seconds) => {
  const renderer = (() => {
    const gl = document.createElement("canvas").getContext("webgl");
    const info = gl?.getExtension("WEBGL_debug_renderer_info");
    return info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : "unknown";
  })();
  const frames = [];
  let longTasks = 0;
  let longTaskMs = 0;
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      longTasks += 1;
      longTaskMs += entry.duration;
    }
  });
  observer.observe({ type: "longtask", buffered: false });
  const ws = { ...window.__ws };
  await new Promise((resolve) => {
    let last = performance.now();
    const end = last + seconds * 1000;
    const frame = (now) => {
      frames.push(now - last);
      last = now;
      if (now < end) requestAnimationFrame(frame);
      else resolve();
    };
    requestAnimationFrame(frame);
  });
  observer.disconnect();
  frames.sort((a, b) => a - b);
  const at = (q) => frames[Math.round((frames.length - 1) * q)];
  return {
    renderer,
    cabsOnMap: document.querySelector(".legend__moving")?.textContent ?? "",
    fps: frames.length / seconds,
    frameP50: at(0.5),
    frameP95: at(0.95),
    frameMax: frames[frames.length - 1],
    slowFramesPct: (frames.filter((f) => f > 33.4).length * 100) / frames.length,
    longTasks,
    longTaskMsPerS: longTaskMs / seconds,
    heapMb: performance.memory ? performance.memory.usedJSHeapSize / 1e6 : null,
    wsMessagesPerS: (window.__ws.messages - ws.messages) / seconds,
    wsKbPerS: (window.__ws.bytes - ws.bytes) / 1024 / seconds,
    missedEventsPerMin: ((window.__ws.missed - ws.missed) * 60) / seconds,
  };
}, seconds);

await browser.close();
console.log(JSON.stringify({ url: url.toString(), seconds, ...result }, null, 2));
