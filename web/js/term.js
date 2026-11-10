// restty (Ghostty's VT core in WebAssembly) behind a PTY transport with no process behind it:
// what we write is the "program output", and what restty sends back (replies to queries,
// keys, mouse reports) is the "program input". Nothing else touches the terminal.

import { Restty, getBuiltinTheme } from "../vendor/restty.esm.js";

const dec = new TextDecoder();

export class Term {
  constructor(root, { theme, fontSize, fonts, renderer = "auto" }) {
    this.size = null;
    this.onInput = () => {};
    this.onResize = () => {};
    let connected;
    this.ready = new Promise((r) => (connected = r));
    const self = this;
    const transport = {
      connect(opts) {
        self.cb = opts.callbacks;
        self.cb.onConnect?.();
        connected();
      },
      disconnect() {
        self.cb = null;
      },
      sendInput(data) {
        self.onInput(data);
        return true;
      },
      resize(cols, rows, meta = {}) {
        const s = { cols, rows, cellW: meta.cellW ?? 0, cellH: meta.cellH ?? 0, widthPx: meta.widthPx, heightPx: meta.heightPx };
        const changed = !self.size || s.cols !== self.size.cols || s.rows !== self.size.rows || s.cellW !== self.size.cellW || s.cellH !== self.size.cellH;
        self.size = s;
        if (changed) self.onResize(s);
        return true;
      },
      isConnected() {
        return !!self.cb;
      },
    };
    this.restty = new Restty({
      root,
      surface: { searchUi: false, shortcuts: false, defaultContextMenu: false, createInitialPane: { focus: false } },
      terminal: { renderer, fontSize, theme: getBuiltinTheme(theme), fonts, showResizeOverlay: false },
      services: { ptyTransport: transport },
    });
    this.restty.connectPty("kittychart-demo://local");
  }

  async start() {
    await this.ready;
    const t0 = performance.now();
    while (!this.size && performance.now() - t0 < 5000) await new Promise((r) => setTimeout(r, 30));
    // Let the first layout settle (fonts load, the pane gets its final size).
    await new Promise((r) => setTimeout(r, 250));
    return this.size;
  }

  /** Program output: a string or bytes. */
  write(data) {
    if (!this.cb) return;
    this.cb.onData(typeof data === "string" ? data : dec.decode(data));
  }

  setTheme(name) {
    this.restty.applyTheme(getBuiltinTheme(name), name);
  }

  focus() {
    this.restty.focus();
  }
}
