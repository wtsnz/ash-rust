// Records a CPU profile of the page for a few seconds and prints where the main thread's
// time goes, by function. Run against a live API like bench-browser.mjs.
//
//   node scripts/profile-browser.mjs --url http://127.0.0.1:4392 [--wall] [--seconds 8]

import puppeteer from "puppeteer-core";

const flag = (name, fallback) => {
  const at = process.argv.indexOf(`--${name}`);
  if (at === -1) return fallback;
  const next = process.argv[at + 1];
  return next === undefined || next.startsWith("--") ? true : next;
};
const url = new URL(flag("url", "http://localhost:4321"));
if (flag("wall", false)) url.searchParams.set("mode", "wall");
const seconds = Number(flag("seconds", 8));

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  headless: "new",
  args: ["--ignore-gpu-blocklist", "--enable-gpu-rasterization"],
});
const page = await browser.newPage();
await page.setViewport({ width: 1600, height: 900, deviceScaleFactor: 1 });
await page.goto(url.toString(), { waitUntil: "load", timeout: 120_000 });
await new Promise((resolve) => setTimeout(resolve, 12_000));
const cdp = await page.createCDPSession();
await cdp.send("Profiler.enable");
await cdp.send("Profiler.setSamplingInterval", { interval: 200 });
await cdp.send("Profiler.start");
await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
const { profile } = await cdp.send("Profiler.stop");
await browser.close();

const byId = new Map(profile.nodes.map((node) => [node.id, node]));
const self = new Map();
const total = profile.samples.length;
for (const id of profile.samples) {
  const node = byId.get(id);
  const { functionName, url: source, lineNumber } = node.callFrame;
  const file = source.split("/").pop()?.split("?")[0] ?? "";
  const key = `${functionName || "(anonymous)"}  ${file}:${lineNumber + 1}`;
  self.set(key, (self.get(key) ?? 0) + 1);
}
const rows = [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 40);
for (const [key, count] of rows) console.log(`${((count * 100) / total).toFixed(1).padStart(5)}%  ${key}`);
