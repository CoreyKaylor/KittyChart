// Smoke test for the built demo site (target/web): boots it in headless Chromium and checks
// that both halves work end to end. Run: node web/test/smoke.mjs [site-dir]
//
// Uses playwright-core and its Chromium (`npx playwright-core install chromium`, or set
// CHROMIUM=/path/to/chrome). WebGPU is usually missing headless; restty falls back to WebGL2
// (SwiftShader), which is what this exercises.

import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { extname, join, resolve } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
let chromium;
try {
  ({ chromium } = require("playwright-core"));
} catch {
  ({ chromium } = require(resolve(import.meta.dirname, "../../tools/reference/node_modules/playwright-core")));
}

const site = resolve(process.argv[2] ?? join(import.meta.dirname, "../../target/web"));
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".json": "application/json", ".svg": "image/svg+xml", ".ttf": "font/ttf", ".txt": "text/plain" };
const server = createServer(async (req, res) => {
  let p = join(site, decodeURIComponent(new URL(req.url, "http://x").pathname));
  try {
    if ((await stat(p)).isDirectory()) p = join(p, "index.html");
    const body = await readFile(p);
    res.writeHead(200, { "content-type": types[extname(p)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404).end("not found");
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/`;

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const errors = [];
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") errors.push(`console: ${m.text()}`);
});

let failed = 0;
async function check(name, fn) {
  try {
    await fn();
    console.log(`ok    ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL  ${name}\n      ${String(e.message ?? e).split("\n")[0]}`);
  }
}
const kc = (f) => page.evaluate(f);
const until = (f, what, timeout = 20000) =>
  page.waitForFunction(f, null, { timeout, polling: 100 }).catch(() => {
    throw new Error(`timed out waiting for ${what}`);
  });

/** Count distinct colors in an element's screenshot (decoded in the page). */
async function colors(selector) {
  const png = (await page.locator(selector).screenshot()).toString("base64");
  return page.evaluate(async (b64) => {
    const img = await createImageBitmap(await (await fetch(`data:image/png;base64,${b64}`)).blob());
    const c = new OffscreenCanvas(img.width, img.height);
    const g = c.getContext("2d");
    g.drawImage(img, 0, 0);
    const d = g.getImageData(0, 0, img.width, img.height).data;
    const seen = new Set();
    for (let i = 0; i < d.length; i += 4 * 7) seen.add((d[i] >> 3) << 10 | (d[i + 1] >> 3) << 5 | (d[i + 2] >> 3));
    return seen.size;
  }, png);
}

await page.goto(base);

await check("boots and detects kitty graphics in restty", async () => {
  await until(() => document.documentElement.dataset.ready === "1", "the boot sequence");
  const d = await kc(() => window.__kc.state.detected);
  if (!d?.kitty) throw new Error(`detected: ${JSON.stringify(d)}`);
  if (!d.colors) throw new Error("no theme colors reported");
});

await check("autoplay opens `kittychart view` and frames reach the terminal", async () => {
  await until(() => window.__kc.state.mode === "view" && window.__kc.state.frames > 3, "viewer frames");
  await page.waitForTimeout(1500);
  const n = await colors("#term");
  if (n < 12) throw new Error(`terminal shows only ${n} colors`);
});

await check("the canvas pane draws the chart", async () => {
  if ((await kc(() => window.__kc.state.webFrames)) < 3) throw new Error("no canvas frames");
  const n = await colors("#web");
  if (n < 6) throw new Error(`canvas shows only ${n} colors`);
});

await check("hovering the canvas hovers the same element in the terminal", async () => {
  const box = await page.locator("#web").boundingBox();
  const before = await kc(() => window.__kc.state.frames);
  // Over the last bar of the revenue chart (Dec), then hold still.
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5);
  await page.mouse.move(box.x + box.width - 24, box.y + box.height * 0.6, { steps: 4 });
  // Both settle on the same elements (the terminal's follows a moment later).
  await until(() => window.__kc.state.termActive.length > 0 && String(window.__kc.web.active) === String(window.__kc.state.termActive), "matching hovers", 5000)
    .catch(async (e) => {
      const [web, term] = await kc(() => [window.__kc.web.active, window.__kc.state.termActive]);
      throw new Error(`canvas ${web} vs terminal ${term}`);
    });
  await until(() => window.__kc.state.frames > 0, "a frame");
  const after = await kc(() => window.__kc.state.frames);
  if (after <= before) throw new Error("no terminal frame after the hover");
  const wire = await page.locator("#wire").innerText();
  if (!/pointer move \(from canvas\)/i.test(wire)) throw new Error("the wire shows no synthesized pointer report");
});

await check("leaving the canvas clears the terminal's hover", async () => {
  await page.mouse.move(5, 5);
  await until(() => window.__kc.state.termActive.length === 0, "the hover to clear");
});

await check("editing the config animates both views", async () => {
  await page.locator("#edit summary").click();
  const [w0, t0] = await kc(() => [window.__kc.state.webFrames, window.__kc.state.frames]);
  await page.locator("#editor").evaluate((el) => {
    el.value = el.value.replace(/"data": \[\s*38,/, '"data": [ 5,');
    el.dispatchEvent(new Event("input"));
  });
  // Both redraw repeatedly (an animation, not a single redraw). Headless Chromium renders
  // restty's WebGL in software on the main thread, so the canvas gets few frames here.
  await page.waitForFunction(([w, t]) => window.__kc.state.webFrames - w >= 2 && window.__kc.state.frames - t >= 3, [w0, t0], { timeout: 5000, polling: 100 })
    .catch(async () => {
      const [w1, t1] = await kc(() => [window.__kc.state.webFrames, window.__kc.state.frames]);
      throw new Error(`frames: canvas +${w1 - w0}, terminal +${t1 - t0}`);
    });
  if (!(await page.locator("#edit-status").innerText()).startsWith("valid")) throw new Error("editor status not valid");
});

await check("the shell runs an inline text chart", async () => {
  await kc(() => window.__kc.shell.type("kittychart traffic.json --protocol braille", { delay: 1, startDelay: 1 }));
  await until(() => window.__kc.state.mode === "shell" && window.__kc.state.inlineBytes > 500, "inline output");
});

await check("q leaves the viewer back to the prompt", async () => {
  await kc(() => window.__kc.shell.type("kittychart view latency.json", { delay: 1, startDelay: 1 }));
  await until(() => window.__kc.state.mode === "view", "the viewer");
  await page.locator("#term").click();
  await page.keyboard.press("q");
  await until(() => window.__kc.state.mode === "shell", "the prompt");
});

await check("the docs and gallery are served", async () => {
  for (const path of ["docs/", "docs/gallery.html", "docs/python.html", "docs/gallery/bar-grouped-dark.png"]) {
    const r = await page.request.get(base + path);
    if (!r.ok()) throw new Error(`${path}: HTTP ${r.status()}`);
  }
  const gallery = await (await page.request.get(base + "docs/gallery.html")).text();
  if ((gallery.match(/<img /g) ?? []).length < 40) throw new Error("the gallery page has too few images");
});

await check("no page errors", async () => {
  const real = errors.filter((e) => !/No available adapters/.test(e));
  if (real.length) throw new Error(real.join(" | "));
});

await browser.close();
server.close();
console.log(failed ? `\n${failed} check(s) failed` : "\nall checks passed");
process.exit(failed ? 1 : 0);
