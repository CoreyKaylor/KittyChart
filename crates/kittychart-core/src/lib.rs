//! KittyChart's chart engine: a port of Chart.js's model (config, option resolution, scales,
//! layout, controllers, elements and plugins) onto the [`kittychart_canvas::Canvas`] API.
//!
//! The engine is pure computation over a canvas: no I/O, no clocks, no globals beyond
//! immutable defaults. The same config always produces the same pixels.

pub mod arc;
pub mod chart;
pub mod color;
pub mod config;
pub mod defaults;
pub mod draw;
pub mod element;
pub mod format;
pub mod layout;
pub mod math;
pub mod options;
pub mod plugins;
pub mod scale;
pub mod theme;

pub use chart::Chart;
pub use config::{ChartConfig, ChartType, ConfigError, Label};
pub use theme::{TerminalColors, Theme};
