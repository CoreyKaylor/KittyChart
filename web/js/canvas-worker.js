// The browser half, off the main thread: KittyChart's WebChart drawing into an OffscreenCanvas.
// Same Rust code as the terminal, rasterized by tiny-skia in WebAssembly; the pixels go to the
// canvas with putImageData. No Chart.js anywhere.

import init, { WebChart } from "../pkg/kittychart_wasm.js";

const wasm = await init();
let canvas = null;
let ctx = null;
let chart = null;
let size = [0, 0, 1];
let lastActive = "";

const raf = self.requestAnimationFrame?.bind(self) ?? ((f) => setTimeout(() => f(performance.now()), 16));

function emitActive() {
  const pairs = Array.from(chart.active());
  const key = pairs.join(",");
  if (key !== lastActive) {
    lastActive = key;
    postMessage({ type: "active", pairs });
  }
}

function loop(now) {
  if (chart) {
    const t0 = performance.now();
    if (chart.frame(now)) {
      const w = chart.pixel_width();
      const h = chart.pixel_height();
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      const px = new Uint8ClampedArray(wasm.memory.buffer, chart.pixels_ptr(), chart.pixels_len());
      ctx.putImageData(new ImageData(px, w, h), 0, 0);
      postMessage({ type: "frame", ms: performance.now() - t0, w, h });
    }
  }
  raf(loop);
}

onmessage = ({ data: m }) => {
  try {
    switch (m.type) {
      case "init":
        canvas = m.canvas;
        ctx = canvas.getContext("2d");
        raf(loop);
        break;
      case "load":
        chart?.free();
        size = [m.w, m.h, m.dpr];
        chart = new WebChart(m.json, m.w, m.h, m.dpr);
        lastActive = "";
        break;
      case "resize":
        size = [m.w, m.h, m.dpr];
        chart?.resize(m.w, m.h, m.dpr);
        break;
      case "update":
        chart?.update(m.json);
        break;
      case "replay":
        chart?.replay();
        break;
      case "toggle":
        chart?.toggle(m.item);
        break;
      case "pointer":
        if (!chart) break;
        chart.pointer(m.x, m.y);
        postMessage({ type: "cursor", legend: chart.over_legend(m.x, m.y) });
        emitActive();
        break;
      case "leave":
        if (!chart) break;
        chart.leave();
        emitActive();
        break;
      case "click": {
        if (!chart) break;
        const item = chart.click(m.x, m.y);
        if (item >= 0) postMessage({ type: "legend", item });
        break;
      }
      case "highlight":
        if (!chart) break;
        chart.highlight(new Uint32Array(m.pairs));
        lastActive = m.pairs.join(",");
        break;
    }
  } catch (e) {
    postMessage({ type: "error", message: String(e?.message ?? e) });
  }
};

postMessage({ type: "ready" });
