//! KittyChart's chart engine: a port of Chart.js's model (config, option resolution, scales,
//! layout, controllers, elements and plugins) onto the [`kittychart_canvas::Canvas`] API.
//!
//! The engine is pure computation over a canvas: no I/O, no clocks, no globals beyond
//! immutable defaults. The same config always produces the same pixels.
#![recursion_limit = "256"] // the `json!` defaults table in defaults.rs

pub mod arc;
pub mod chart;
pub mod color;
pub mod config;
pub mod decimation;
pub mod defaults;
pub mod draw;
pub mod element;
pub mod filler;
pub mod format;
pub mod gradient;
pub mod interaction;
pub mod layout;
pub mod math;
pub mod options;
pub mod plugins;
pub mod radial;
pub mod scale;
pub mod script;
pub mod theme;
pub mod time;
pub mod tooltip;

pub use chart::Chart;
pub use config::{ChartConfig, ChartType, ConfigError, Label};
pub use theme::{TerminalColors, Theme};
