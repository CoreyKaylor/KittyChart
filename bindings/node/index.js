'use strict'

const native = require('./binding.js')

/**
 * JSON.stringify replacer: typed arrays become plain arrays, BigInts numbers. (Dates already
 * become ISO strings, which time scales parse, and NaN/Infinity become null: a gap.)
 */
function replacer(_key, value) {
  if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
    return Array.from(value, (x) => (typeof x === 'bigint' ? Number(x) : x))
  }
  if (typeof value === 'bigint') return Number(value)
  return value
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

  /** @param {object} config A Chart.js configuration: `{ type, data, options }`. */
  constructor(config) {
    if (!isPlainObject(config)) throw new TypeError('kittychart: Chart needs a config object { type, data, options }')
    this.#set(config)
  }

  #set(config) {
    const json = JSON.stringify(config, replacer)
    native.validate(json)
    this.#config = config
    this.#json = json
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
    return native.renderPng(this.#json, opts)
  }

  /** An SVG document of the same chart (text as outlines, no fonts needed). */
  toSvg(opts) {
    return native.renderSvg(this.#json, opts)
  }

  /** `{ width, height, data }`: non-premultiplied RGBA8 pixels. */
  toRgba(opts) {
    return native.renderRgba(this.#json, opts)
  }

  /** Block and Braille characters with truecolor ANSI escapes. */
  toText(opts = {}) {
    const { cols = 80, rows, mode, colors } = opts
    return native.renderText(this.#json, cols, rows, mode, colors)
  }

  /**
   * Draw the chart at the cursor: kitty graphics, then sixel, then text, whichever the
   * terminal supports. Returns what was drawn: `{ mode, cols, rows }`.
   */
  show(opts) {
    const p = native.prepare(this.#json, opts)
    if (p.target === 'tty') native.writeTty(p.data)
    else process.stdout.write(p.data)
    return { mode: p.mode, cols: p.cols, rows: p.rows }
  }

  /** Render for a known terminal (see `probe`) without any I/O: for TUI frameworks. */
  prepare(info, opts) {
    return native.prepareFor(this.#json, info, opts)
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
