// Record real Chart.js hit-testing for KittyChart's interaction tests: for each fixture,
// getElementsAtEventForMode over a grid of positions, every mode, intersect on and off.
// Usage: node tools/reference/interactions.mjs <fixtures-dir> <out-dir> fixture.json…
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { chromium } from 'playwright-core';

const [fixturesDir, outDir, ...names] = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const fonts = join(here, '../../crates/kittychart-canvas/fonts');
const b64 = (f) => readFileSync(join(fonts, f)).toString('base64');
const faces = ['Inter', 'Helvetica Neue', 'Helvetica', 'Arial']
  .flatMap((n) => [
    `@font-face{font-family:'${n}';src:url(data:font/ttf;base64,${b64('Inter-Regular.ttf')});font-weight:100 599}`,
    `@font-face{font-family:'${n}';src:url(data:font/ttf;base64,${b64('Inter-Bold.ttf')});font-weight:600 900}`,
  ])
  .join('\n');

const browser = await chromium.launch({ args: ['--font-render-hinting=none'], env: { ...process.env, TZ: 'UTC' } });
const page = await browser.newPage();
await page.setContent(`<!doctype html><style>${faces} body{margin:0}</style><body></body>`);
await page.addScriptTag({ path: join(dirname(require.resolve('chart.js')), 'chart.umd.js') });
await page.addScriptTag({ path: join(dirname(require.resolve('chartjs-adapter-date-fns')), 'chartjs-adapter-date-fns.bundle.min.js') });
await page.evaluate(async () => {
  for (const f of ['Inter', 'Helvetica Neue', 'Helvetica', 'Arial']) {
    await document.fonts.load(`12px "${f}"`);
    await document.fonts.load(`bold 12px "${f}"`);
  }
});
mkdirSync(outDir, { recursive: true });
for (const name of names) {
  const config = JSON.parse(readFileSync(join(fixturesDir, name), 'utf8'));
  const kc = config._kc || {};
  delete config._kc;
  const result = await page.evaluate(({ config, kc }) => {
    const width = kc.width ?? 512;
    const height = kc.height ?? width / (config.options?.aspectRatio ?? 2);
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    document.body.replaceChildren(canvas);
    config.options = { ...config.options, animation: false, responsive: false, devicePixelRatio: kc.dpr ?? 1 };
    const chart = new Chart(canvas, config);
    const queries = [];
    const step = 12;
    for (const mode of ['index', 'dataset', 'point', 'nearest', 'x', 'y']) {
      for (const intersect of [true, false]) {
        for (let y = step / 2; y < height; y += step) {
          for (let x = step / 2; x < width; x += step) {
            const els = chart.getElementsAtEventForMode({ x, y, native: {} }, mode, { intersect }, true);
            if (els.length) queries.push([mode, intersect, x, y, els.map((e) => [e.datasetIndex, e.index])]);
          }
        }
      }
    }
    chart.destroy();
    return { width, height, step, queries };
  }, { config, kc });
  const out = join(outDir, name.replace(/\//g, '-'));
  writeFileSync(out, JSON.stringify(result));
  console.log(`${name}: ${result.queries.length} hits`);
}
await browser.close();
