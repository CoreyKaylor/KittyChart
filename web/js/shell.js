// A small pretend shell for the demo terminal. It exists only to give you a prompt: every
// chart it prints comes from KittyChart (in the worker), byte for byte what the CLI writes.

const C = {
  reset: "\x1b[0m",
  dim: "\x1b[2m",
  bold: "\x1b[1m",
  red: "\x1b[31m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
  blue: "\x1b[34m",
  magenta: "\x1b[35m",
  cyan: "\x1b[36m",
};

const PROMPT = `${C.cyan}~/demo${C.reset} ${C.magenta}❯${C.reset} `;
const PROTOCOLS = ["auto", "kitty", "sixel", "text", "blocks", "half", "braille"];
const COMMANDS = ["kittychart", "ls", "cat", "clear", "help"];

function words(line) {
  return line.trim().split(/\s+/).filter(Boolean);
}

/** Pretty-print JSON with ANSI colors, as `cat` of a formatted file with highlighting would. */
function colorJson(text) {
  let pretty;
  try {
    pretty = JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
  return pretty
    .replace(/("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false|null)\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g, (m, str, colon, lit, num) => {
      if (str) return colon ? `${C.blue}${str}${C.reset}${colon}` : `${C.green}${str}${C.reset}`;
      if (lit) return `${C.magenta}${lit}${C.reset}`;
      return `${C.yellow}${num}${C.reset}`;
    })
    .replace(/\n/g, "\r\n");
}

export class Shell {
  /**
   * @param {object} h host hooks: write(str), files() -> Map name -> json text,
   *   inline({json, protocol, cols, placeholders}), view({name, json, protocol}),
   *   viewing() -> bool, quitView(), clear(), onCommand(line)
   */
  constructor(h) {
    this.h = h;
    this.line = "";
    this.history = [];
    this.hist = -1;
    this.typing = null;
    this.busy = false;
  }

  prompt() {
    this.line = "";
    this.h.write(PROMPT);
  }

  /** Keystrokes from the terminal while no program runs. */
  key(data) {
    if (this.typing) this.cancelTyping();
    if (this.busy) return;
    for (let i = 0; i < data.length; i++) {
      const ch = data[i];
      if (ch === "\x1b") {
        const rest = data.slice(i);
        const m = /^\x1b\[([AB])/.exec(rest) || /^\x1bO([AB])/.exec(rest);
        if (m) {
          this.recall(m[1] === "A" ? 1 : -1);
          i += m[0].length - 1;
        } else {
          // Skip other escape sequences (arrows left/right, function keys…).
          const s = /^\x1b(\[[0-9;?]*[ -/]*[@-~]|O.|.)/.exec(rest);
          i += (s ? s[0].length : 1) - 1;
        }
      } else if (ch === "\r" || ch === "\n") {
        this.enter();
      } else if (ch === "\x7f" || ch === "\b") {
        if (this.line) {
          this.line = this.line.slice(0, -1);
          this.h.write("\b \b");
        }
      } else if (ch === "\x03") {
        this.h.write("^C\r\n");
        this.prompt();
      } else if (ch === "\x0c") {
        this.h.clear();
        this.h.write(PROMPT + this.line);
      } else if (ch === "\t") {
        this.complete();
      } else if (ch >= " ") {
        this.line += ch;
        this.h.write(ch);
      }
    }
  }

  recall(dir) {
    if (!this.history.length) return;
    this.hist = Math.max(-1, Math.min(this.history.length - 1, this.hist + dir));
    const next = this.hist < 0 ? "" : this.history[this.history.length - 1 - this.hist];
    this.h.write("\b \b".repeat(this.line.length) + next);
    this.line = next;
  }

  complete() {
    const parts = this.line.split(" ");
    const last = parts[parts.length - 1];
    const pool = parts.length === 1 ? COMMANDS : [...this.h.files().keys(), "view", "--protocol", "--cols", "--placeholders", ...PROTOCOLS];
    const hits = pool.filter((w) => w.startsWith(last));
    if (hits.length === 1) {
      const add = hits[0].slice(last.length) + " ";
      this.line += add;
      this.h.write(add);
    } else if (hits.length > 1) {
      this.h.write(`\r\n${hits.join("  ")}\r\n${PROMPT}${this.line}`);
    }
  }

  enter() {
    const line = this.line;
    this.h.write("\r\n");
    if (line.trim()) {
      this.history.push(line);
      this.hist = -1;
    }
    this.run(line);
  }

  /** Type `line` at the prompt, a key at a time, then run it. */
  async type(line, { delay = 38, startDelay = 250 } = {}) {
    this.cancelTyping();
    if (this.h.viewing()) await this.h.quitView();
    if (this.busy) return false;
    return new Promise((resolve) => {
      let i = 0;
      const step = () => {
        if (i < line.length) {
          this.line += line[i];
          this.h.write(line[i]);
          i++;
          this.typing = setTimeout(step, delay * (0.6 + Math.random() * 0.8));
        } else {
          this.typing = null;
          this.enter();
          resolve(true);
        }
      };
      this.typing = setTimeout(step, startDelay);
      this.cancelTyped = () => resolve(false);
    });
  }

  cancelTyping() {
    if (!this.typing) return;
    clearTimeout(this.typing);
    this.typing = null;
    this.cancelTyped?.();
  }

  /** A program finished: back to the prompt. */
  done() {
    this.busy = false;
    this.prompt();
  }

  run(line) {
    const argv = words(line);
    this.h.onCommand?.(line);
    if (!argv.length) return this.prompt();
    const [cmd, ...args] = argv;
    const out = (s) => this.h.write(s.replace(/\n/g, "\r\n") + "\r\n");
    switch (cmd) {
      case "help":
        out(
          `${C.bold}Commands${C.reset}\n` +
            `  ls                          list the chart configs\n` +
            `  cat FILE                    show one\n` +
            `  kittychart FILE [--protocol P] [--cols N] [--placeholders]\n` +
            `                              draw it here (P: ${PROTOCOLS.join(", ")})\n` +
            `  kittychart view FILE [--protocol P]\n` +
            `                              explore it: hover, click the legend, ←/→, 1-9, r, s, q\n` +
            `  clear                       clear the screen (or Ctrl-L)\n` +
            `${C.dim}Everything kittychart prints is the real library's output, compiled to WebAssembly.${C.reset}`,
        );
        return this.prompt();
      case "ls":
        out([...this.h.files().keys()].map((f) => `${C.green}${f}${C.reset}`).join("  "));
        return this.prompt();
      case "cat": {
        if (!args.length) {
          out("cat: missing file operand");
          return this.prompt();
        }
        for (const f of args) {
          const text = this.h.files().get(f);
          out(text === undefined ? `cat: ${f}: No such file or directory` : colorJson(text));
        }
        return this.prompt();
      }
      case "clear":
        this.h.clear();
        return this.prompt();
      case "kittychart":
        return this.kittychart(args, out);
      default:
        out(`kcsh: command not found: ${cmd}`);
        return this.prompt();
    }
  }

  kittychart(args, out) {
    const view = args[0] === "view";
    if (view) args = args.slice(1);
    let file = null;
    let protocol = "auto";
    let cols = null;
    let placeholders = false;
    for (let i = 0; i < args.length; i++) {
      let a = args[i];
      let value = null;
      if (a.startsWith("--") && a.includes("=")) [a, value] = [a.slice(0, a.indexOf("=")), a.slice(a.indexOf("=") + 1)];
      const need = () => value ?? args[++i];
      if (a === "--protocol") protocol = need();
      else if (a === "--cols" && !view) cols = Number(need());
      else if (a === "--placeholders" && !view) placeholders = true;
      else if (a === "-V" || a === "--version") {
        out("kittychart-cli 0.0.1 (wasm32)");
        return this.prompt();
      } else if (a === "-h" || a === "--help") {
        out(
          view
            ? "Usage: kittychart view [--protocol <PROTOCOL>] <FILE>"
            : "Usage: kittychart [--protocol <PROTOCOL>] [--cols <COLS>] [--placeholders] <FILE>\n       kittychart view <FILE>",
        );
        return this.prompt();
      } else if (a.startsWith("-")) {
        out(`error: unexpected argument '${a}' found\n\nUsage: kittychart${view ? " view" : ""} [OPTIONS] <FILE>`);
        return this.prompt();
      } else file = a;
    }
    if (!PROTOCOLS.includes(protocol)) {
      out(`error: invalid value '${protocol}' for '--protocol <PROTOCOL>'\n  [possible values: ${PROTOCOLS.join(", ")}]`);
      return this.prompt();
    }
    if (!file) {
      out(`error: the following required arguments were not provided:\n  <FILE>\n\nUsage: kittychart${view ? " view" : ""} [OPTIONS] <FILE>`);
      return this.prompt();
    }
    const json = this.h.files().get(file);
    if (json === undefined) {
      out(`kittychart: ${file}: No such file or directory (os error 2)`);
      return this.prompt();
    }
    if (protocol === "sixel") {
      out(`${C.dim}(restty has no sixel support; drawing with kitty graphics instead)${C.reset}`);
      protocol = "auto";
    }
    if (view) {
      this.busy = true;
      this.h.view({ name: file, json, protocol });
    } else {
      this.busy = true;
      this.h.inline({ json, protocol, cols: Number.isFinite(cols) && cols > 0 ? cols : null, placeholders });
    }
  }
}
