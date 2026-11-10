/// <reference types="node" />

/** Chart types KittyChart draws (any string is accepted and validated at runtime). */
export type ChartType =
  | 'line'
  | 'bar'
  | 'pie'
  | 'doughnut'
  | 'polarArea'
  | 'radar'
  | 'scatter'
  | 'bubble'
  | (string & {})

/** Data values: numbers, typed arrays, `{x, y}` points, `[x, y]` pairs, Dates, null for gaps. */
export type DataValues = ArrayLike<unknown> | readonly unknown[]

/**
 * A Chart.js configuration. Structurally compatible with Chart.js's `ChartConfiguration`, so
 * configs typed for Chart.js can be passed as they are.
 */
export interface ChartConfiguration {
  type: ChartType
  data: {
    labels?: DataValues
    datasets: ReadonlyArray<{ data: DataValues; label?: string; type?: ChartType } | object>
  }
  options?: object
  plugins?: unknown
}

export interface RenderOptions {
  /** Logical (CSS) width in pixels. Default 800. */
  width?: number
  /** Logical height. Default: from `options.aspectRatio`. */
  height?: number
  /** Device pixel ratio: the bitmap is `width·scale × height·scale`. Default 1. */
  scale?: number
  /** Any CSS color. Default: transparent. */
  background?: string
}

/** Terminal colors as `[r, g, b]`. */
export interface TerminalColors {
  background: number[]
  foreground: number[]
  /** ANSI colors 0–15. */
  palette?: number[][]
}

export type TextMode = 'text' | 'blocks' | 'half' | 'braille'
export type Protocol = 'auto' | 'kitty' | 'sixel' | TextMode

export interface TextOptions {
  /** Width in cells. Default 80. */
  cols?: number
  /** Height in cells. Default: from the aspect ratio. */
  rows?: number
  mode?: TextMode
  /** Derive chart colors from these terminal colors. */
  colors?: TerminalColors
}

export interface ShowOptions {
  /** Default: `$KITTYCHART_PROTOCOL`, else 'auto' (kitty graphics, then sixel, then text). */
  protocol?: Protocol
  /** Width in cells. Default: the terminal width. */
  cols?: number
  /** Height in cells. Default: from the aspect ratio. */
  rows?: number
  /** Logical width/height in CSS pixels (override cols/rows). */
  width?: number
  height?: number
  /** Device pixels per CSS pixel. Default: chart text about as tall as terminal text. */
  scale?: number
  background?: string
  /** kitty: draw with Unicode placeholders (automatic inside tmux). */
  placeholders?: boolean
  /** Skip terminal detection. */
  force?: boolean
  /** 'terminal' (default): colors follow the terminal theme; 'chartjs': Chart.js defaults. */
  colors?: 'terminal' | 'chartjs'
  /** kitty image id. Default: a fresh one per chart drawn. */
  imageId?: number
}

/** What the terminal is and supports, from `probe()`. */
export interface TerminalInfo {
  cols: number
  rows: number
  /** Cell size in device pixels. */
  cellWidth: number
  cellHeight: number
  kitty: boolean
  sixel: boolean
  tmux: boolean
  colors?: TerminalColors
}

/** A chart rendered for a terminal, not yet written. */
export interface Prepared {
  mode: Exclude<Protocol, 'auto'>
  /** Escapes or ANSI text, ending with a newline. */
  data: Buffer
  target: 'stdout' | 'tty'
  cols: number
  rows: number
  /** `data.subarray(0, graphicsLen)` is the graphics escapes alone. */
  graphicsLen: number
  imageId?: number
}

export interface Shown {
  mode: Exclude<Protocol, 'auto'>
  cols: number
  rows: number
}

export class Chart {
  constructor(config: ChartConfiguration)
  static fromJSON(text: string): Chart
  /** The configuration, as given. */
  readonly config: ChartConfiguration
  /** The configuration as Chart.js JSON (typed arrays converted). */
  toJSON(): ChartConfiguration
  /** Merge changes into the configuration; `data` replaces the data. */
  update(patch: Partial<ChartConfiguration>): this
  toPng(opts?: RenderOptions): Buffer
  toRgba(opts?: RenderOptions): { width: number; height: number; data: Buffer }
  toText(opts?: TextOptions): string
  /** Draw at the cursor with the best protocol the terminal supports. */
  show(opts?: ShowOptions): Shown
  /** Render for a known terminal without I/O (for TUI frameworks). */
  prepare(info: TerminalInfo, opts?: ShowOptions): Prepared
}

/** Draw a Chart.js config in the terminal. */
export function show(config: ChartConfiguration | Chart, opts?: ShowOptions): Shown

/** Ask the terminal what it supports. `null` without a terminal. */
export function probe(timeoutMs?: number): TerminalInfo | null

export const version: string
