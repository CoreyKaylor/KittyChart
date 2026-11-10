// Explore a chart full-screen: hover, click legend items, arrows, r replays, q quits.
//   node examples/interact.js
const { Chart } = require('..')

new Chart({
  type: 'bar',
  data: {
    labels: ['Q1', 'Q2', 'Q3', 'Q4'],
    datasets: [
      { label: 'Revenue', data: [12, 19, 7, 15], backgroundColor: (ctx) => (ctx.raw > 10 ? '#ff0000' : '#ff8080') },
      { label: 'Costs', data: [8, 11, 14, 9], backgroundColor: '#0000ff' },
    ],
  },
  options: { interaction: { mode: 'index', intersect: false } },
}).interact()
