import { CanvasChart } from "./web-chart.js";
import { Term } from "./term.js";
import { Shell } from "./shell.js";

const $ = (id) => document.getElementById(id);
const enc = new TextEncoder();
const dec = new TextDecoder();

const PRESETS = [
  ["revenue", "Revenue"],
  ["latency", "Latency"],
  ["traffic", "Traffic"],
  ["skills", "Scorecard"],
  ["gradient", "Gradient"],
];

const state = {
  file: "revenue.json",
  protocol: "auto",
  mode: "boot", // boot | shell | detecting | view | inline
  viewing: null, // the file the viewer shows
  driver: null, // "web" | "term": which pane the pointer is in (the other one follows)
  detected: null,
  frames: 0,
  bytesDown: 0,
  webFrames: 0,
  termActive: [],
  inlineBytes: 0,
};

const files = new Map();
await Promise.all(
  PRESETS.map(async ([name]) => {
    const r = await fetch(`configs/${name}.json`);
    files.set(`${name}.json`, await r.text());
  }),
);

// GitHub link, when the build knows the repository.
const repo = document.querySelector('meta[name="kc-repo"]')?.content;
if (repo) {
  $("gh").href = repo;
  $("gh").hidden = false;
}

// ---------------------------------------------------------------- the wire
const wireList = $("wire");
const fmtBytes = (n) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB` : `${(n / 1048576).toFixed(1)} MB`);
const escape = (s) => s.replace(/\x1b/g, "\\e").replace(/[\x00-\x1f]/g, (c) => `\\x${c.charCodeAt(0).toString(16).padStart(2, "0")}`);

function wire(dir, title, detail) {
  const li = document.createElement("li");
  li.className = dir;
  li.innerHTML = `<b>${dir === "down" ? "↓" : "↑"} ${title}</b>`;
  const span = document.createElement("span");
  span.className = "x";
  span.textContent = detail;
  li.append(span);
  wireList.prepend(li);
  while (wireList.children.length > 14) wireList.lastChild.remove();
  $("wire-total").textContent = `${state.frames} frames · ${fmtBytes(state.bytesDown)}`;
}

/** A one-line summary of output bytes: the kitty command, or text. */
function describeOutput(bytes, frame) {
  const head = dec.decode(bytes.subarray(0, 220));
  const m = /\x1b_G([^;\x1b]*)/.exec(head);
  if (m) {
    const keys = Object.fromEntries(m[1].split(",").map((kv) => kv.split("=")));
    return ["kitty frame", `a=${keys.a} f=${keys.f}${keys.o ? " o=z" : ""} · ${keys.s}×${keys.v} · ${fmtBytes(bytes.length)}${frame ? ` · ${frame.ms.toFixed(0)} ms` : ""}`];
  }
  return [frame?.mode === "text" ? "text frame" : "output", `ANSI cells · ${fmtBytes(bytes.length)}${frame ? ` · ${frame.ms.toFixed(0)} ms` : ""}`];
}

function describeInput(bytes) {
  const s = dec.decode(bytes);
  const m = /\x1b\[<(\d+);(\d+);(\d+)([Mm])/.exec(s);
  if (m) {
    const kind = +m[1] & 32 ? "pointer move" : m[4] === "m" ? "release" : "press";
    return [kind, `${escape(m[0])}  (${m[2]}, ${m[3]}) px`];
  }
  if (s === "\x1b[O") return ["focus out", "\\e[O"];
  if (s === "\x1b[I") return ["focus in", "\\e[I"];
  return ["key", escape(s)];
}

// ---------------------------------------------------------------- the canvas
const web = new CanvasChart($("web"), {
  onActive: (pairs) => {
    state.driver = "web";
    if (state.mode === "view") worker.postMessage({ type: "point", pairs: pairs.slice(0, 2) });
  },
  onLegend: (item) => {
    // The viewer's number keys toggle the same legend item.
    if (state.mode === "view" && item < 9) worker.postMessage({ type: "key", text: String(item + 1) });
  },
  onFrame: ({ ms, w, h }) => {
    state.webFrames++;
    $("web-stat").textContent = `${w}×${h} px · ${ms.toFixed(1)} ms`;
  },
});
web.load(files.get(state.file));
$("web").addEventListener("pointerenter", () => (state.driver = "web"));

// ---------------------------------------------------------------- the terminal
const fontSize = window.matchMedia("(max-width: 820px)").matches ? 12 : 15;
// WebGL2 by default: restty's WebGPU renderer draws nothing on some GPU drivers (seen on
// Asahi Linux) without failing, so its "auto" can't fall back. ?renderer=webgpu|auto to try it.
const renderer = new URLSearchParams(location.search).get("renderer") ?? "webgl2";
const term = new Term($("term"), { theme: $("theme").value, fontSize, renderer, fonts: ["fonts/JetBrainsMono-Regular.ttf"] });
$("term").addEventListener("pointerenter", () => (state.driver = "term"));

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
const workerReady = new Promise((resolve) => {
  worker.addEventListener("message", function once(e) {
    if (e.data.type === "ready") {
      worker.removeEventListener("message", once);
      resolve();
    }
  });
});

const shell = new Shell({
  write: (s) => term.write(s),
  files: () => files,
  inline: (opts) => {
    state.mode = "inline";
    worker.postMessage({ type: "inline", ...opts });
  },
  view: ({ name, json, protocol }) => {
    state.mode = "view";
    state.viewing = name;
    if (name !== state.file) selectFile(name, { fromShell: true });
    worker.postMessage({ type: "view", json, protocol });
    web.replay(); // both open with the same animation
  },
  viewing: () => state.mode === "view",
  quitView,
  clear: () => term.write("\x1b[H\x1b[2J\x1b[3J"),
});

// Leave the viewer; resolves once it has restored the screen.
let exitWaiters = [];
function quitView() {
  if (state.mode !== "view") return Promise.resolve();
  return new Promise((resolve) => {
    exitWaiters.push(resolve);
    worker.postMessage({ type: "quit" });
  });
}

let lastDownWire = 0;
worker.onmessage = ({ data: m }) => {
  switch (m.type) {
    case "write": {
      term.write(m.bytes);
      if (state.mode === "inline") state.inlineBytes += m.bytes.length;
      if (m.frame) {
        state.frames++;
        state.bytesDown += m.bytes.length;
        // Log at most a few frames a second; the totals count all of them.
        const now = performance.now();
        if (now - lastDownWire > 240) {
          lastDownWire = now;
          wire("down", ...describeOutput(m.bytes, m.frame));
        } else {
          $("wire-total").textContent = `${state.frames} frames · ${fmtBytes(state.bytesDown)}`;
        }
      }
      break;
    }
    case "wire":
      if (m.dir === "in") wire("up", ...((d) => [m.synthetic ? `${d[0]} (from canvas)` : d[0], d[1]])(describeInput(m.bytes)));
      break;
    case "active":
      state.termActive = m.pairs;
      if (state.driver !== "web") web.highlight(m.pairs);
      break;
    case "detected":
      onDetected(m);
      break;
    case "inlined":
      state.mode = "shell";
      shell.done();
      break;
    case "exited":
      if (state.mode === "view") {
        state.mode = "shell";
        state.viewing = null;
        shell.done();
      }
      for (const r of exitWaiters.splice(0)) r();
      break;
    case "png": {
      const url = URL.createObjectURL(new Blob([m.bytes], { type: "image/png" }));
      const a = Object.assign(document.createElement("a"), { href: url, download: `${(state.viewing ?? "chart.json").replace(/\.json$/, "")}.png` });
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 2000);
      wire("down", "save (s)", `PNG ${fmtBytes(m.bytes.length)}, downloaded`);
      break;
    }
    case "error":
      console.error("kittychart:", m.message);
      term.write(`\r\n\x1b[31mkittychart: ${m.message}\x1b[0m\r\n`);
      if (state.mode === "inline" || state.mode === "view") {
        state.mode = "shell";
        shell.done();
      }
      break;
  }
};

// Everything the terminal sends: replies to queries, keys, mouse and focus reports.
let lastUpWire = 0;
term.onInput = (data) => {
  if (state.mode === "detecting") {
    const kitty = /\x1b_Gi=\d+;(\w+)/.exec(data);
    if (kitty) wire("up", "kitty graphics reply", `\\e_Gi=19267;${kitty[1]}`);
    else if (/\x1b\[\?[\d;]*c/.test(data)) wire("up", "device attributes", escape(/\x1b\[\?[\d;]*c/.exec(data)[0]));
  }
  if (state.mode === "detecting" || state.mode === "view") {
    const now = performance.now();
    const pointer = data.startsWith("\x1b[<");
    const show = state.mode === "view" && (!pointer || now - lastUpWire > 180);
    if (show) lastUpWire = now;
    worker.postMessage({ type: "reply", bytes: enc.encode(data), wire: show });
  } else if (state.mode === "shell") {
    shell.key(data);
  }
};
term.onResize = (s) => worker.postMessage({ type: "resize", cols: s.cols, rows: s.rows, cellW: s.cellW, cellH: s.cellH });

// ---------------------------------------------------------------- detection and boot
function detect() {
  const s = term.size;
  state.mode = "detecting";
  wire("down", "capability queries", "\\e_Gi=19267,a=q,… \\e[16t \\e]10;? \\e]11;? \\e]4;0..15;? \\e[c");
  return new Promise((resolve) => {
    detectDone = (m) => resolve({ m });
    worker.postMessage({ type: "detect", cols: s.cols, rows: s.rows, cellW: s.cellW, cellH: s.cellH });
  });
}
let detectDone = null;
function onDetected(m) {
  state.detected = m;
  $("late-note").hidden = !m.late;
  detectDone?.(m);
  detectDone = null;
}

function bootReport(m) {
  const ok = (b) => (b ? "\x1b[32m✓\x1b[0m" : "\x1b[31m✗\x1b[0m");
  const dim = (s) => `\x1b[2m${s}\x1b[0m`;
  term.write(
    [
      dim(`restty 0.3.0 · Ghostty's VT core in WebAssembly · ${m.cols}×${m.rows} cells`),
      dim("kittychart asked this terminal what it supports:"),
      `  kitty graphics  ${ok(m.kitty)}${m.late ? dim("  (its reply came after DA1¹)") : ""}`,
      `  sixel           ${ok(m.sixel)}`,
      `  cell size       ${m.cellW}×${m.cellH} px`,
      `  theme colors    ${ok(m.colors)}${m.palette ? dim("  fg, bg and 16-color palette") : ""}`,
      "",
    ].join("\r\n") + "\r\n",
  );
}

async function boot() {
  await workerReady;
  const s = await term.start();
  worker.postMessage({ type: "resize", cols: s.cols, rows: s.rows, cellW: s.cellW, cellH: s.cellH });
  const { m } = await detect();
  state.mode = "shell";
  bootReport(m);
  shell.prompt();
  document.documentElement.dataset.ready = "1";
  await shell.type(`kittychart view ${state.file}`, { startDelay: 2200 });
}

// ---------------------------------------------------------------- controls
const presetBox = $("presets");
for (const [name, label] of PRESETS) {
  const b = document.createElement("button");
  b.type = "button";
  b.role = "tab";
  b.dataset.file = `${name}.json`;
  b.innerHTML = `${label} <span class="ext">${name}.json</span>`;
  b.addEventListener("click", () => selectFile(`${name}.json`));
  presetBox.append(b);
}

function selectFile(name, { fromShell = false } = {}) {
  state.file = name;
  for (const b of presetBox.children) b.setAttribute("aria-selected", String(b.dataset.file === name));
  web.load(files.get(name));
  showInEditor();
  if (!fromShell && state.mode !== "boot" && state.mode !== "detecting") runView();
}

function runView() {
  const p = state.protocol === "auto" ? "" : ` --protocol ${state.protocol}`;
  shell.type(`kittychart view ${state.file}${p}`, { delay: 22, startDelay: 120 });
}

for (const b of $("protocol").querySelectorAll("button")) {
  b.addEventListener("click", () => {
    if (b.disabled) return;
    state.protocol = b.dataset.p;
    for (const o of $("protocol").querySelectorAll("button")) o.setAttribute("aria-checked", String(o === b));
    runView();
  });
}

$("theme").addEventListener("change", async () => {
  // An inline chart still printing: let it finish, so its output and the detection replies
  // don't interleave.
  while (state.mode === "inline") await new Promise((r) => setTimeout(r, 20));
  term.setTheme($("theme").value);
  // Ask again: KittyChart takes its default colors from what the terminal reports.
  if (state.mode === "boot" || state.mode === "detecting") return;
  const was = state.viewing;
  await quitView();
  const { m } = await detect();
  state.mode = "shell";
  if (was) {
    shell.type(`kittychart view ${was}${state.protocol === "auto" ? "" : ` --protocol ${state.protocol}`}`, { delay: 18, startDelay: 60 });
  } else {
    term.write(`\r\n\x1b[2mtheme changed; detected again: theme colors ${m.colors ? "✓" : "✗"}\x1b[0m\r\n`);
    shell.prompt();
  }
});

const CHIPS = [
  "ls",
  "cat revenue.json",
  "kittychart latency.json",
  "kittychart traffic.json --protocol braille",
  "kittychart view gradient.json",
  "help",
];
for (const c of CHIPS) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = c;
  b.addEventListener("click", () => {
    if (state.mode === "boot" || state.mode === "detecting") return;
    shell.type(c, { delay: 20, startDelay: 80 });
    term.focus();
  });
  $("chips").append(b);
}

// Both views at once: the canvas directly, the terminal through the key the viewer reads.
$("replay").addEventListener("click", () => {
  web.replay();
  if (state.mode === "view") worker.postMessage({ type: "key", text: "r" });
});
$("toggle1").addEventListener("click", () => {
  web.toggle(0);
  if (state.mode === "view") worker.postMessage({ type: "key", text: "1" });
});

// Random data: both views animate to it.
$("shuffle").addEventListener("click", () => {
  const cfg = JSON.parse(files.get(state.file));
  for (const ds of cfg.data.datasets) {
    ds.data = ds.data.map((v) => {
      if (typeof v !== "number") return v;
      const f = 0.45 + Math.random() * 1.1;
      return Math.max(1, Math.round(v * f));
    });
  }
  setConfig(JSON.stringify(cfg, null, 2) + "\n");
  showInEditor();
});

function setConfig(text) {
  files.set(state.file, text);
  web.update(text);
  if (state.mode === "view" && state.viewing === state.file) worker.postMessage({ type: "update", json: text });
}

// ---------------------------------------------------------------- editor
const editor = $("editor");
function showInEditor() {
  editor.value = files.get(state.file);
  $("edit-file").textContent = state.file;
  $("edit-name").textContent = state.file;
  setStatus(true);
}
function setStatus(ok, msg) {
  const s = $("edit-status");
  s.className = ok ? "ok" : "err";
  s.textContent = ok ? "valid · both views follow" : msg;
}
let editTimer = 0;
editor.addEventListener("input", () => {
  clearTimeout(editTimer);
  editTimer = setTimeout(() => {
    try {
      JSON.parse(editor.value);
    } catch (e) {
      setStatus(false, e.message.replace(/^JSON\.parse: /, ""));
      return;
    }
    try {
      setConfig(editor.value);
      setStatus(true);
    } catch (e) {
      setStatus(false, String(e.message ?? e));
    }
  }, 350);
});

// ---------------------------------------------------------------- code tabs
for (const b of document.querySelectorAll(".code-tabs button")) {
  b.addEventListener("click", () => {
    for (const o of document.querySelectorAll(".code-tabs button")) o.setAttribute("aria-selected", String(o === b));
    for (const p of document.querySelectorAll(".code pre")) p.hidden = p.dataset.tab !== b.dataset.tab;
  });
}

// For the smoke test (web/test/smoke.mjs): read-only peeks at the demo's state.
window.__kc = { state, shell, web };

selectFile(state.file, { fromShell: true });
boot().catch((e) => {
  console.error(e);
  term.write(`\r\n\x1b[31mdemo failed to start: ${e.message ?? e}\x1b[0m\r\n`);
});
