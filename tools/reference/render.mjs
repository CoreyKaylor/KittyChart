// Render every KittyChart fixture with real Chart.js in headless Chromium, for parity reports.
// Usage: node tools/reference/render.mjs <fixtures-dir> <out-dir>
//
// Chromium is the reference because browsers are where people know Chart.js's look.
// (skia-canvas, for example, places the `middle` text baseline differently from every browser.)
// The bundled Inter font is mapped onto Chart.js's default font stack, so remaining
// differences come from the engine, not from fonts. One-time setup: `npx playwright-core install chromium`.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { chromium } from 'playwright-core';

const [fixturesDir, outDir] = process.argv.slice(2);
if (!fixturesDir || !outDir) {
  console.error('usage: render.mjs <fixtures-dir> <out-dir>');
  process.exit(2);
}

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const fonts = join(here, '../../crates/kittychart-canvas/fonts');
const b64 = (f) => readFileSync(join(fonts, f)).toString('base64');
const regular = b64('Inter-Regular.ttf');
const bold = b64('Inter-Bold.ttf');
const faces = ['Inter', 'Helvetica Neue', 'Helvetica', 'Arial']
  .flatMap((name) => [
    `@font-face{font-family:'${name}';src:url(data:font/ttf;base64,${regular});font-weight:100 599}`,
    `@font-face{font-family:'${name}';src:url(data:font/ttf;base64,${bold});font-weight:600 900}`,
  ])
  .join('\n');

function* walk(dir) {
  for (const e of readdirSync(dir).sort()) {
    const p = join(dir, e);
    if (statSync(p).isDirectory()) yield* walk(p);
    else if (p.endsWith('.json')) yield p;
  }
}

// Unhinted fonts give fractional glyph advances, as macOS/Windows/HiDPI browsers have. Linux
// Chromium otherwise rounds every advance to whole pixels.
// TZ=UTC: KittyChart's time scales compute in UTC, so the reference does too.
const browser = await chromium.launch({ args: ['--font-render-hinting=none'], env: { ...process.env, TZ: 'UTC' } });
const page = await browser.newPage();
await page.setContent(`<!doctype html><style>${faces} body{margin:0}</style><body></body>`);
await page.addScriptTag({ path: join(dirname(require.resolve('chart.js')), 'chart.umd.js') });
// Time scales need a date adapter; KittyChart mirrors chartjs-adapter-date-fns.
await page.addScriptTag({ path: join(dirname(require.resolve('chartjs-adapter-date-fns')), 'chartjs-adapter-date-fns.bundle.min.js') });
await page.evaluate(async () => {
  for (const fam of ['Inter', 'Helvetica Neue', 'Helvetica', 'Arial']) {
    await document.fonts.load(`12px "${fam}"`);
    await document.fonts.load(`bold 12px "${fam}"`);
  }
});

let n = 0;
for (const file of walk(fixturesDir)) {
  const config = JSON.parse(readFileSync(file, 'utf8'));
  const kc = config._kc || {};
  delete config._kc;
  const dataUrl = await page.evaluate(({ config, kc }) => {
    const width = kc.width ?? 512;
    const dpr = kc.dpr ?? 1;
    const height = kc.height ?? width / (config.options?.aspectRatio ?? 2);
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    document.body.replaceChildren(canvas);
    config.options = { ...config.options, animation: false, responsive: false, devicePixelRatio: dpr };
    // KittyChart's declarative gradients ({type, x0…, stops}) become the scriptable options
    // Chart.js users write: (ctx) => ctx.chart.ctx.createLinearGradient(…).
    const coord = (v, a, dflt) => {
      if (v === undefined) return dflt;
      if (typeof v === 'number') return v;
      const k = String(v).replace(/^chartArea\./, '');
      const m = { left: a.left, right: a.right, top: a.top, bottom: a.bottom,
        centerX: (a.left + a.right) / 2, centerY: (a.top + a.bottom) / 2 };
      return k in m ? m[k] : Number(k);
    };
    const toGradient = (g) => (ctx) => {
      const a = ctx.chart.chartArea;
      if (!a) return undefined;
      const c = ctx.chart.ctx;
      const cx = (a.left + a.right) / 2, cy = (a.top + a.bottom) / 2;
      let grad;
      if (g.type === 'linear') grad = c.createLinearGradient(coord(g.x0, a, a.left), coord(g.y0, a, a.bottom), coord(g.x1, a, a.left), coord(g.y1, a, a.top));
      else if (g.type === 'radial') grad = c.createRadialGradient(coord(g.x0, a, cx), coord(g.y0, a, cy), coord(g.r0, a, 0), coord(g.x1, a, cx), coord(g.y1, a, cy), coord(g.r1, a, Math.min(a.right - a.left, a.bottom - a.top) / 2));
      else grad = c.createConicGradient(g.angle ?? 0, coord(g.x, a, cx), coord(g.y, a, cy));
      for (const s of g.stops) Array.isArray(s) ? grad.addColorStop(s[0], s[1]) : grad.addColorStop(s.offset, s.color);
      return grad;
    };
    const isGradient = (v) => v && typeof v === 'object' && !Array.isArray(v) && ['linear', 'radial', 'conic'].includes(v.type) && Array.isArray(v.stops);
    const convert = (v) => {
      if (isGradient(v)) return toGradient(v);
      if (Array.isArray(v)) return v.map(convert);
      if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, convert(x)]));
      return v;
    };
    config.data.datasets = convert(config.data.datasets);
    config.options = convert(config.options);
    config.plugins = [{
      id: 'kcBackground',
      beforeDraw(chart) {
        const { ctx } = chart;
        ctx.save();
        ctx.fillStyle = kc.background ?? '#ffffff';
        ctx.fillRect(0, 0, chart.width, chart.height);
        ctx.restore();
      },
    }];
    const chart = new Chart(canvas, config);
    const url = canvas.toDataURL('image/png');
    chart.destroy();
    return url;
  }, { config, kc });
  const out = join(outDir, relative(fixturesDir, file).replace(/\.json$/, '.png'));
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, Buffer.from(dataUrl.split(',')[1], 'base64'));
  n++;
}
const version = await page.evaluate(() => Chart.version);
await browser.close();
console.log(`rendered ${n} fixtures with Chart.js ${version} in ${browser.version?.() ?? 'Chromium'}`);
