// The terminal side of the demo, off the main thread: KittyChart compiled to WebAssembly,
// talking bytes with the terminal. Everything here is what the `kittychart` CLI does over a
// TTY: detect the terminal, draw inline charts, run `kittychart view`.
//
// Messages in:  detect {cols, rows, cellW, cellH} · reply {bytes} · resize {...}
//               inline {json, protocol, cols, placeholders} · view {json, protocol}
//               update {json} · point {pairs} · key {text} · quit
// Messages out: write {bytes, frame?} · detected {...} · active {pairs} · wire {...}
//               png {bytes} · exited · error {message}

import init, { Detect, Viewer, inline } from "../pkg/kittychart_wasm.js";

await init();

const enc = new TextEncoder();
const post = (m, transfer) => postMessage(m, transfer ?? []);

let size = { cols: 80, rows: 24, cellW: 8, cellH: 16 };
let detect = null;
let detectTimer = 0;
let daSeen = false;
let info = null;
let viewer = null;
let lastActive = "";
let loop = 0;

function write(bytes, frame) {
  const u = bytes instanceof Uint8Array ? bytes : enc.encode(bytes);
  post({ type: "write", bytes: u, frame }, [u.buffer]);
}

function finishDetect() {
  clearTimeout(detectTimer);
  detectTimer = 0;
  if (!detect) return;
  info = detect.info(size.cols, size.rows, size.cellW, size.cellH);
  post({
    type: "detected",
    kitty: info.kitty(),
    sixel: info.sixel(),
    colors: info.colors(),
    palette: info.palette(),
    late: detect.late_kitty(),
    best: info.best(),
    cols: info.cols(),
    rows: info.rows(),
    cellW: info.cell_width(),
    cellH: info.cell_height(),
  });
  detect.free();
  detect = null;
}

function startLoop() {
  if (loop) return;
  const tick = () => {
    if (!viewer) {
      loop = 0;
      return;
    }
    const t0 = performance.now();
    const f = viewer.frame(t0);
    if (f) write(f, { ms: performance.now() - t0, bytes: f.length, mode: viewer.mode() });
    loop = setTimeout(tick, 8);
  };
  loop = setTimeout(tick, 0);
}

function stopViewer() {
  if (!viewer) return;
  write(new Uint8Array([...viewer.cleanup(), ...Viewer.leave()]));
  viewer.free();
  viewer = null;
  lastActive = "";
  post({ type: "exited" });
}

function reportActive() {
  if (!viewer) return;
  const pairs = Array.from(viewer.active());
  const key = pairs.join(",");
  if (key !== lastActive) {
    lastActive = key;
    post({ type: "active", pairs });
  }
}

// Bytes for the running viewer, from the terminal (a real pointer or key) or synthesized by
// the page (the linked hover): parsed by KittyChart's input parser, handled by Session.
function input(bytes, synthetic) {
  if (!viewer) return;
  const action = viewer.input(bytes);
  if (synthetic) post({ type: "wire", dir: "in", bytes, synthetic: true });
  if (action === 3) {
    const png = viewer.take_png();
    if (png) post({ type: "png", bytes: png }, [png.buffer]);
  } else if (action === 2) {
    stopViewer();
    return;
  }
  reportActive();
}

onmessage = ({ data: m }) => {
  try {
    switch (m.type) {
      case "detect": {
        size = { cols: m.cols, rows: m.rows, cellW: m.cellW, cellH: m.cellH };
        detect = new Detect();
        daSeen = false;
        write(detect.queries());
        // No DA1 within a second: a terminal that answers nothing.
        detectTimer = setTimeout(finishDetect, 1000);
        break;
      }
      case "reply": {
        const bytes = m.bytes;
        if (detect) {
          if (detect.feed(bytes)) {
            clearTimeout(detectTimer);
            const probe = detect.info(1, 1, 0, 0);
            const kitty = probe.kitty();
            probe.free();
            if (kitty) {
              // Everything is in (a terminal that answers in order never waits here).
              finishDetect();
            } else if (!daSeen) {
              // DA1 arrived without a kitty reply. restty sends that reply after DA1 (its
              // JavaScript layer answers DA1 first); give it up to half a second.
              daSeen = true;
              detectTimer = setTimeout(finishDetect, 500);
            }
          }
        } else if (viewer) {
          if (m.wire) post({ type: "wire", dir: "in", bytes });
          input(bytes, false);
        }
        break;
      }
      case "resize": {
        size = { cols: m.cols, rows: m.rows, cellW: m.cellW, cellH: m.cellH };
        viewer?.resize(m.cols, m.rows, m.cellW, m.cellH);
        break;
      }
      case "inline": {
        try {
          write(inline(m.json, info, m.protocol, m.cols ?? undefined, !!m.placeholders));
        } finally {
          post({ type: "inlined" });
        }
        break;
      }
      case "view": {
        stopViewer();
        viewer = new Viewer(m.json, info, m.protocol);
        viewer.resize(size.cols, size.rows, size.cellW, size.cellH);
        write(Viewer.enter());
        startLoop();
        break;
      }
      case "update": {
        viewer?.update(m.json);
        break;
      }
      case "point": {
        // The pointer is over these elements in the other view: send the pointer report a
        // terminal would send for the same element here, or a focus-out when it left.
        if (!viewer) break;
        const [ds, index] = m.pairs;
        const report = ds === undefined ? "\x1b[O" : viewer.pointer_report(ds, index) ?? "\x1b[O";
        input(enc.encode(report), true);
        break;
      }
      case "key": {
        input(enc.encode(m.text), true);
        break;
      }
      case "quit": {
        stopViewer();
        break;
      }
    }
  } catch (e) {
    post({ type: "error", message: String(e?.message ?? e) });
  }
};

post({ type: "ready" });
