// Type-checked by `tsc -p __test__/tsconfig.json` (not run): the public typings accept
// Chart.js's own configuration types.
import type { ChartConfiguration as ChartJsConfiguration } from 'chart.js'
import { Chart, show, probe, type ChartConfiguration, type ShowOptions } from '..'

const fromChartJs: ChartJsConfiguration<'bar'> = {
  type: 'bar',
  data: { labels: ['a'], datasets: [{ label: 'x', data: [1], backgroundColor: 'red', borderRadius: 4 }] },
  options: { plugins: { legend: { position: 'bottom' } }, scales: { y: { beginAtZero: true } } },
}
const a = new Chart(fromChartJs)

const line: ChartConfiguration = {
  type: 'line',
  data: { labels: [new Date()], datasets: [{ label: 'y', data: new Float64Array([1]), fill: true }] },
}
const opts: ShowOptions = { protocol: 'auto', cols: 60, colors: 'terminal' }
const shown = new Chart(line).show(opts)
const mode: 'kitty' | 'sixel' | 'text' | 'blocks' | 'half' | 'braille' = shown.mode
const png: Buffer = a.toPng({ width: 400, scale: 2 })
const info = probe()
if (info) a.prepare(info, { placeholders: true }).data.subarray(0, 1)
show(fromChartJs, { protocol: 'text' })
void mode
void png

// @ts-expect-error: a config needs data
new Chart({ type: 'bar' })
// @ts-expect-error: unknown protocol
a.show({ protocol: 'png' })
