// Draw a chart in the terminal: kitty graphics, sixel or text, whichever it supports.
//   node examples/show.js
const { Chart } = require('..')

const hours = Array.from({ length: 24 }, (_, h) => `${h}:00`)
const load = Float64Array.from(hours, (_, h) => 40 + 30 * Math.sin((h - 6) / 3.8) + (h % 5))

const chart = new Chart({
  type: 'line',
  data: {
    labels: hours,
    datasets: [{ label: 'CPU %', data: load, fill: true, tension: 0.3, pointRadius: 0 }],
  },
  options: { plugins: { title: { display: true, text: 'Node → Chart.js → terminal' } } },
})
chart.show({ cols: 70 })
