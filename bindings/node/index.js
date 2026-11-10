'use strict'

const native = require('./binding.js')

/**
 * A JSON.stringify replacer: typed arrays become plain arrays, BigInts numbers. (Dates already
 * become ISO strings, which time scales parse, and NaN/Infinity become null: a gap.)
 * Functions (scriptable options) become `{"__kc_fn": id}` markers, registered in `fns`.
 */
function makeReplacer(fns) {
  return function replacer(_key, value) {
    if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
      return Array.from(value, (x) => (typeof x === 'bigint' ? Number(x) : x))
    }
    if (typeof value === 'bigint') return Number(value)
    if (typeof value === 'function') {
      if (!fns) throw new TypeError('kittychart: functions are only allowed in chart configs')
      let id = [...fns.entries()].find(([, f]) => f === value)?.[0]
      if (id === undefined) {
        id = `f${fns.size}`
        fns.set(id, value)
      }
      return { __kc_fn: id }
    }
    return value
  }
}

/** The native side's callback: `(id, contextJson) => resultJson`. */
function makeDispatch(fns) {
  return (id, ctxJson) => {
    const ctx = JSON.parse(ctxJson)
    const args = ctx.__args
    delete ctx.__args
    const f = fns.get(id)
    const result = args ? f(...args) : f(ctx)
    return JSON.stringify(result === undefined ? null : result, makeReplacer(null))
  }
}

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v) && !ArrayBuffer.isView(v) && !(v instanceof Date)
}

function merge(base, patch) {
  const out = { ...base }
  for (const [k, v] of Object.entries(patch)) {
    out[k] = isPlainObject(v) && isPlainObject(out[k]) ? merge(out[k], v) : v
  }
  return out
}

/** A Chart.js chart, drawn in the terminal or rendered to PNG. */
class Chart {
  #config
  #json
  #dispatch

  /** @param {object} config A Chart.js configuration: `{ type, data, options }`. */
  constructor(config) {
    if (!isPlainObject(config)) throw new TypeError('kittychart: Chart needs a config object { type, data, options }')
    this.#set(config)
  }

  #set(config) {
    const fns = new Map()
    const json = JSON.stringify(config, makeReplacer(fns))
    native.validate(json)
    this.#config = config
    this.#json = json
    this.#dispatch = fns.size ? makeDispatch(fns) : undefined
  }

  /** Parse a Chart.js configuration from JSON text. */
  static fromJSON(text) {
    return new Chart(JSON.parse(text))
  }

  /** The configuration, as given. */
  get config() {
    return this.#config
  }

  /** The configuration as Chart.js JSON. */
  toJSON() {
    return JSON.parse(this.#json)
  }

  /**
   * Merge changes into the configuration (nested objects merge; arrays and values replace).
   * `data`, when given, replaces the data entirely.
   */
  update(patch) {
    const { data, ...rest } = patch ?? {}
    const next = merge(this.#config, rest)
    if (data !== undefined) next.data = data
    this.#set(next)
    return this
  }

  /** PNG bytes. `width`/`height` in CSS pixels (height from the aspect ratio), `scale` = DPR. */
  toPng(opts) {
    return native.renderPng(this.#json, opts, this.#dispatch)
  }

  /** An SVG document of the same chart (text as outlines, no fonts needed). */
  toSvg(opts) {
    return native.renderSvg(this.#json, opts, this.#dispatch)
  }

  /** `{ width, height, data }`: non-premultiplied RGBA8 pixels. */
  toRgba(opts) {
    return native.renderRgba(this.#json, opts, this.#dispatch)
  }

  /** Block and Braille characters with truecolor ANSI escapes. */
  toText(opts = {}) {
    const { cols = 80, rows, mode, colors } = opts
    return native.renderText(this.#json, cols, rows, mode, colors, this.#dispatch)
  }

  /**
   * Draw the chart at the cursor: kitty graphics, then sixel, then text, whichever the
   * terminal supports. Returns what was drawn: `{ mode, cols, rows }`.
   */
  show(opts) {
    const p = native.prepare(this.#json, opts, this.#dispatch)
    if (p.target === 'tty') native.writeTty(p.data)
    else process.stdout.write(p.data)
    return { mode: p.mode, cols: p.cols, rows: p.rows }
  }

  /** Render for a known terminal (see `probe`) without any I/O: for TUI frameworks. */
  prepare(info, opts) {
    return native.prepareFor(this.#json, info, opts, this.#dispatch)
  }

  /**
   * Explore the chart full-screen until `q`: hover for tooltips, click legend items to hide
   * datasets, arrows to step, `r` to replay the animation, `s` to save a PNG. With
   * `{ watch: 'chart.json' }`, the chart animates to that file's contents when it changes.
   * Blocks until the user quits.
   */
  interact(opts = {}) {
    const { fastTransfer = true, ...rest } = opts
    native.interact(this.#json, { ...rest, noFastTransfer: !fastTransfer }, this.#dispatch)
  }
}

/** Draw a Chart.js config in the terminal. See `Chart#show`. */
function show(config, opts) {
  return (config instanceof Chart ? config : new Chart(config)).show(opts)
}

// Plain `exports.x =` assignments, so ESM `import { Chart } from 'kittychart'` finds every
// name (Node detects CommonJS named exports statically).
exports.Chart = Chart
exports.show = show
exports.probe = native.probe
exports.version = native.version()
