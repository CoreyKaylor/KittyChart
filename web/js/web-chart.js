// The canvas pane: forwards pointer input to canvas-worker.js, where KittyChart draws into an
// OffscreenCanvas. Keeps the main thread free for the terminal.

export class CanvasChart {
  constructor(canvas, { onActive, onLegend, onFrame, onError } = {}) {
    this.canvas = canvas;
    this.onActive = onActive ?? (() => {});
    this.onLegend = onLegend ?? (() => {});
    this.onFrame = onFrame ?? (() => {});
    this.onError = onError ?? ((m) => console.error("kittychart canvas:", m));
    this.active = [];
    this.pointerInside = false;
    this.worker = new Worker(new URL("./canvas-worker.js", import.meta.url), { type: "module" });
    this.ready = new Promise((resolve) => {
      this.worker.onmessage = ({ data: m }) => {
        switch (m.type) {
          case "ready":
            resolve();
            break;
          case "frame":
            this.onFrame(m);
            break;
          case "active":
            this.active = m.pairs;
            if (this.pointerInside) this.onActive(m.pairs);
            break;
          case "cursor":
            this.canvas.style.cursor = m.legend ? "pointer" : "default";
            break;
          case "legend":
            this.onLegend(m.item);
            break;
          case "error":
            this.onError(m.message);
            break;
        }
      };
    });
    const off = canvas.transferControlToOffscreen();
    this.send({ type: "init", canvas: off }, [off]);
    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(canvas.parentElement);
    canvas.addEventListener("pointermove", (e) => {
      this.pointerInside = true;
      this.send({ type: "pointer", ...this.pos(e) });
    });
    canvas.addEventListener("pointerleave", () => {
      this.send({ type: "leave" });
      // The leave's "no active elements" still goes to the terminal.
      setTimeout(() => (this.pointerInside = false), 100);
    });
    canvas.addEventListener("pointerdown", (e) => this.send({ type: "click", ...this.pos(e) }));
  }

  /** Post once the worker is listening (it loads WebAssembly first). */
  send(m, transfer) {
    this.ready.then(() => this.worker.postMessage(m, transfer ?? []));
  }

  metrics() {
    const r = this.canvas.parentElement.getBoundingClientRect();
    return { w: Math.max(50, Math.floor(r.width)), h: Math.max(50, Math.floor(r.height)), dpr: Math.min(window.devicePixelRatio || 1, 2) };
  }

  pos(e) {
    const r = this.canvas.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  load(json) {
    this.size = this.metrics();
    this.active = [];
    this.send({ type: "load", json, ...this.size });
  }

  resize() {
    if (!this.size) return;
    const m = this.metrics();
    if (m.w === this.size.w && m.h === this.size.h && m.dpr === this.size.dpr) return;
    this.size = m;
    this.send({ type: "resize", ...m });
  }

  update(json) {
    this.send({ type: "update", json });
  }

  replay() {
    this.send({ type: "replay" });
  }

  toggle(item) {
    this.send({ type: "toggle", item });
  }

  /** Mirror another view's hover (only while the pointer isn't here). */
  highlight(pairs) {
    if (this.pointerInside) return;
    this.send({ type: "highlight", pairs });
  }
}
