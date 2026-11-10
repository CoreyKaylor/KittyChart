const test = require('node:test')
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const zlib = require('node:zlib')

const kittychart = require('..')
const { Chart } = kittychart

const ROOT = path.resolve(__dirname, '../../..')
const BAR = { type: 'bar', data: { labels: ['Q1', 'Q2', 'Q3'], datasets: [{ label: 'Revenue', data: [12, 19, 7] }] } }

/** Minimal decoder for the 8-bit RGBA PNGs used here. */
function decodePng(buf) {
  let pos = 8
  let width = 0
  let height = 0
  const idat = []
  while (pos < buf.length) {
    const n = buf.readUInt32BE(pos)
    const kind = buf.toString('ascii', pos + 4, pos + 8)
    const body = buf.subarray(pos + 8, pos + 8 + n)
    if (kind === 'IHDR') {
      width = body.readUInt32BE(0)
      height = body.readUInt32BE(4)
      assert.deepEqual([body[8], body[9]], [8, 6], '8-bit RGBA')
    } else if (kind === 'IDAT') idat.push(body)
    pos += 12 + n
  }
  const raw = zlib.inflateSync(Buffer.concat(idat))
  const stride = width * 4
  const out = Buffer.alloc(stride * height)
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)]
    for (let i = 0; i < stride; i++) {
      const x = raw[y * (stride + 1) + 1 + i]
      const a = i >= 4 ? out[y * stride + i - 4] : 0
      const b = y > 0 ? out[(y - 1) * stride + i] : 0
      const c = i >= 4 && y > 0 ? out[(y - 1) * stride + i - 4] : 0
      let pred = 0
      if (f === 1) pred = a
      else if (f === 2) pred = b
      else if (f === 3) pred = (a + b) >> 1
      else if (f === 4) {
        const p = a + b - c
        const pa = Math.abs(p - a)
        const pb = Math.abs(p - b)
        const pc = Math.abs(p - c)
        pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c
      }
      out[y * stride + i] = (x + pred) & 255
    }
  }
  return { width, height, data: out }
}

test('renders PNG and RGBA', () => {
  const c = new Chart(BAR)
  const { width, height, data } = c.toRgba({ width: 400, scale: 2 })
  assert.deepEqual([width, height, data.length], [800, 400, 800 * 400 * 4])
  const png = c.toPng({ width: 300, height: 300, background: 'white' })
  const d = decodePng(png)
  assert.deepEqual([d.width, d.height], [300, 300])
  assert.equal(d.data[3], 255)
})

for (const name of ['bar/grouped', 'line/basic', 'arc/pie']) {
  test(`pixels match the Rust golden: ${name}`, () => {
    const cfg = JSON.parse(fs.readFileSync(path.join(ROOT, 'tests/fixtures', `${name}.json`), 'utf8'))
    const kc = cfg._kc ?? {}
    delete cfg._kc
    const img = new Chart(cfg).toRgba({
      width: kc.width ?? 512,
      height: kc.height,
      scale: kc.dpr ?? 1,
      background: kc.background ?? '#ffffff',
    })
    const golden = decodePng(fs.readFileSync(path.join(ROOT, 'tests/goldens/fixtures', `${name}.png`)))
    assert.deepEqual([img.width, img.height], [golden.width, golden.height])
    let diff = 0
    for (let i = 0; i < img.data.length; i++) diff += Math.abs(img.data[i] - golden.data[i])
    assert.ok(diff / img.data.length < 0.5, `mean difference ${diff / img.data.length}`)
  })
}

test('SVG', () => {
  const svg = new Chart(BAR).toSvg({ width: 400 })
  assert.ok(svg.startsWith('<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200"'))
})

test('typed arrays, NaN, BigInt and Dates become Chart.js JSON', () => {
  const c = new Chart({
    type: 'line',
    data: {
      labels: [new Date('2026-01-01T00:00:00Z'), new Date('2026-01-02T00:00:00Z')],
      datasets: [{ data: new Float32Array([1.5, NaN]) }, { data: new BigInt64Array([3n, 4n]) }, { data: [Infinity, 2] }],
    },
    options: { scales: { x: { type: 'time' } } },
  })
  const d = c.toJSON().data
  assert.deepEqual(d.labels, ['2026-01-01T00:00:00.000Z', '2026-01-02T00:00:00.000Z'])
  assert.deepEqual(d.datasets.map((s) => s.data), [[1.5, null], [3, 4], [null, 2]])
  c.toPng({ width: 200 })
})

test('invalid input throws', () => {
  assert.throws(() => new Chart({ type: 'nope', data: {} }))
  assert.throws(() => new Chart('bar'), TypeError)
  assert.throws(() => new Chart(BAR).toPng({ background: 'not-a-color' }), /color/)
  assert.throws(() => new Chart(BAR).toPng({ width: -1 }), /width/)
  assert.throws(() => new Chart(BAR).toText({ mode: 'dots' }), /mode/)
  assert.throws(() => new Chart(BAR).show({ protocol: 'png' }), /protocol/)
  assert.throws(() => new Chart(BAR).show({ colors: 'nope' }), /colors/)
})

test('update merges options and replaces data', () => {
  const c = new Chart({ ...BAR, options: { plugins: { title: { display: true, text: 'A' } } } })
  c.update({ options: { plugins: { title: { text: 'B' } } } })
  assert.deepEqual(c.config.options.plugins.title, { display: true, text: 'B' })
  c.update({ data: { labels: ['x'], datasets: [{ label: 'New', data: [1] }] } })
  assert.match(c.toText({ cols: 40 }), /New/)
  assert.throws(() => c.update({ type: 'nope' }))
  assert.equal(c.config.type, 'bar', 'a failed update leaves the chart unchanged')
})

test('text modes', () => {
  const c = new Chart(BAR)
  for (const mode of ['text', 'blocks', 'half', 'braille']) {
    const t = c.toText({ cols: 50, rows: 12, mode })
    assert.match(t, /Revenue/)
    assert.equal(t.split('\r\n').length - 1, 12)
  }
  const dark = { background: [0, 0, 0], foreground: [255, 255, 255] }
  assert.notEqual(c.toText({ cols: 40, colors: dark }), c.toText({ cols: 40 }))
})

test('prepare renders for a known terminal without I/O', () => {
  const info = { cols: 80, rows: 24, cellWidth: 10, cellHeight: 20, kitty: true, sixel: false, tmux: false }
  const c = new Chart(BAR)
  const k = c.prepare(info, { cols: 40, placeholders: true })
  assert.equal(k.mode, 'kitty')
  assert.equal(k.cols, 40)
  assert.ok(k.data.subarray(0, k.graphicsLen).toString().startsWith('\x1b_Ga=T,U=1,'))
  assert.ok(k.data.subarray(k.graphicsLen).toString().includes('\u{10EEEE}'))
  const s = c.prepare({ ...info, kitty: false, sixel: true }, { cols: 40 })
  assert.equal(s.mode, 'sixel')
  assert.ok(s.data.toString().startsWith('\x1bP'))
  const t = c.prepare({ ...info, kitty: false }, { cols: 40 })
  assert.equal(t.mode, 'text')
})

test('fromJSON and version', () => {
  assert.deepEqual(Chart.fromJSON(JSON.stringify(BAR)).toJSON(), BAR)
  assert.match(kittychart.version, /^\d+\.\d+\.\d+$/)
})

test('ESM named imports work', async () => {
  const m = await import('kittychart')
  for (const name of ['Chart', 'show', 'probe', 'version']) assert.ok(name in m, name)
})

test('function options render like their static values', () => {
  const data = [5, 15, 8, 12]
  const cfg = (backgroundColor) => ({
    type: 'bar',
    data: { labels: ['a', 'b', 'c', 'd'], datasets: [{ label: 'Sales', data, backgroundColor }] },
  })
  const scripted = new Chart(cfg((ctx) => (ctx.raw > 10 ? '#ff0000' : '#0000ff'))).toRgba({ width: 400 })
  const fixed = new Chart(cfg(data.map((v) => (v > 10 ? '#ff0000' : '#0000ff')))).toRgba({ width: 400 })
  assert.ok(scripted.data.equals(fixed.data))
})

test('tick callbacks get Chart.js arguments', () => {
  const money = (value, index, ticks) => {
    assert.equal(ticks[index].value, value)
    return 'USD ' + value
  }
  const c = new Chart({
    type: 'bar',
    data: { labels: ['a', 'b'], datasets: [{ label: 'Sales', data: [5, 15] }] },
    options: { scales: { y: { ticks: { callback: money } } } },
  })
  assert.match(c.toText({ cols: 60, rows: 20 }), /USD 15/)
})

test('callback errors throw', () => {
  const boom = () => {
    throw new Error('bad color')
  }
  const c = new Chart({ type: 'bar', data: { labels: ['a'], datasets: [{ data: [1], backgroundColor: boom }] } })
  assert.throws(() => c.toPng({ width: 200 }), /bad color/)
})
