//! The native half of the `kittychart` Python package. It takes configurations as JSON
//! strings; the Python half (`python/kittychart`) builds them from dicts, numpy, pandas and
//! polars, and does the Rich, Textual and IPython integration. Rendering releases the GIL.

use kittychart::script::Script;
use kittychart::show::{self, Mode, ShowOptions, TerminalInfo};
use kittychart::{CellMode, ChartConfig, RenderOptions, TerminalColors};
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

fn config(json: &str) -> PyResult<ChartConfig> {
    ChartConfig::from_json(json).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// A config whose `{"__kc_fn": id}` markers call `dispatch(id, context_json) -> result_json`
/// (the Python side maps ids to the user's functions).
fn config_with(json: &str, dispatch: Option<Py<PyAny>>) -> PyResult<(ChartConfig, Option<Script>)> {
    let c = config(json)?;
    let Some(d) = dispatch else { return Ok((c, None)) };
    let script = Script::new(move |id, ctx| {
        Python::attach(|py| {
            let r = d.call1(py, (id, ctx.to_string())).map_err(|e| e.to_string())?;
            let text: String = r.extract(py).map_err(|e: PyErr| e.to_string())?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        })
    });
    Ok((c.with_script(script.clone()), Some(script)))
}

/// Raise the first callback error (as RuntimeError), if any.
fn check(script: Option<&Script>) -> PyResult<()> {
    match script.map(Script::take_errors).unwrap_or_default().first() {
        Some(e) => Err(PyRuntimeError::new_err(e.clone())),
        None => Ok(()),
    }
}

fn color(s: Option<&str>) -> PyResult<Option<kittychart::Color>> {
    s.map(|b| kittychart::parse_color(b).ok_or_else(|| PyValueError::new_err(format!("invalid color {b:?}"))))
        .transpose()
}

fn positive(name: &str, v: Option<f32>) -> PyResult<Option<f32>> {
    match v {
        Some(x) if !(x.is_finite() && x > 0.0) => {
            Err(PyValueError::new_err(format!("{name} must be a positive number")))
        }
        v => Ok(v),
    }
}

fn cells(name: &str, v: Option<u32>) -> PyResult<Option<u16>> {
    v.map(|c| {
        u16::try_from(c)
            .ok()
            .filter(|&c| c > 0)
            .ok_or_else(|| PyValueError::new_err(format!("{name} must be 1..=65535")))
    })
    .transpose()
}

fn mode(s: Option<&str>) -> PyResult<Option<Mode>> {
    s.map(|m| m.parse::<Mode>().map_err(PyValueError::new_err)).transpose()
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Auto => "auto",
        Mode::Kitty => "kitty",
        Mode::Sixel => "sixel",
        Mode::Text(CellMode::Auto) => "text",
        Mode::Text(CellMode::Blocks) => "blocks",
        Mode::Text(CellMode::HalfBlocks) => "half",
        Mode::Text(CellMode::Braille) => "braille",
    }
}

/// Check a configuration; raises ValueError with the reason if it's invalid.
#[pyfunction]
fn validate(json: &str) -> PyResult<()> {
    config(json).map(drop)
}

/// Render to PNG bytes.
#[pyfunction]
#[pyo3(signature = (json, width=800.0, height=None, scale=1.0, background=None, dispatch=None))]
fn render_png<'py>(
    py: Python<'py>,
    json: &str,
    width: f32,
    height: Option<f32>,
    scale: f32,
    background: Option<&str>,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<Bound<'py, PyBytes>> {
    let opts = render_options(width, height, scale, background)?;
    let (c, script) = config_with(json, dispatch)?;
    let png = py.detach(|| kittychart::render(c, &opts).to_png());
    check(script.as_ref())?;
    Ok(PyBytes::new(py, &png))
}

fn render_options(
    width: f32,
    height: Option<f32>,
    scale: f32,
    background: Option<&str>,
) -> PyResult<RenderOptions> {
    let width = positive("width", Some(width))?.unwrap_or(800.0);
    let mut opts = RenderOptions::new(width).dpr(positive("scale", Some(scale))?.unwrap_or(1.0));
    opts.height = positive("height", height)?;
    opts.background = color(background)?;
    Ok(opts)
}

/// Render to an SVG document.
#[pyfunction]
#[pyo3(signature = (json, width=800.0, height=None, scale=1.0, background=None, dispatch=None))]
fn render_svg(
    py: Python<'_>,
    json: &str,
    width: f32,
    height: Option<f32>,
    scale: f32,
    background: Option<&str>,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<String> {
    let opts = render_options(width, height, scale, background)?;
    let (c, script) = config_with(json, dispatch)?;
    let svg = py.detach(|| kittychart::render_svg(c, &opts));
    check(script.as_ref())?;
    Ok(svg)
}

/// Render to raw RGBA: `(width, height, bytes)`.
#[pyfunction]
#[pyo3(signature = (json, width=800.0, height=None, scale=1.0, background=None, dispatch=None))]
fn render_rgba<'py>(
    py: Python<'py>,
    json: &str,
    width: f32,
    height: Option<f32>,
    scale: f32,
    background: Option<&str>,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
    let opts = render_options(width, height, scale, background)?;
    let (c, script) = config_with(json, dispatch)?;
    let img = py.detach(|| kittychart::render(c, &opts));
    check(script.as_ref())?;
    Ok((img.width, img.height, PyBytes::new(py, &img.rgba)))
}

fn terminal_colors(d: Option<&Bound<'_, PyDict>>) -> PyResult<Option<TerminalColors>> {
    let Some(d) = d else { return Ok(None) };
    let get = |k: &str| -> PyResult<Option<[u8; 3]>> {
        match d.get_item(k)? {
            Some(v) if !v.is_none() => Ok(Some(v.extract()?)),
            _ => Ok(None),
        }
    };
    let (Some(background), Some(foreground)) = (get("background")?, get("foreground")?) else {
        return Ok(None);
    };
    let palette: Option<Vec<[u8; 3]>> = match d.get_item("palette")? {
        Some(v) if !v.is_none() => Some(v.extract()?),
        _ => None,
    };
    let palette = palette.and_then(|p| <[[u8; 3]; 16]>::try_from(p).ok());
    Ok(Some(TerminalColors { background, foreground, palette }))
}

fn info_to_dict<'py>(py: Python<'py>, i: &TerminalInfo) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("cols", i.cols)?;
    d.set_item("rows", i.rows)?;
    d.set_item("cell_width", i.cell_width)?;
    d.set_item("cell_height", i.cell_height)?;
    d.set_item("kitty", i.kitty)?;
    d.set_item("sixel", i.sixel)?;
    d.set_item("tmux", i.tmux)?;
    match &i.colors {
        Some(c) => {
            let colors = PyDict::new(py);
            colors.set_item("background", c.background)?;
            colors.set_item("foreground", c.foreground)?;
            colors.set_item("palette", c.palette.map(|p| p.to_vec()))?;
            d.set_item("colors", colors)?;
        }
        None => d.set_item("colors", py.None())?,
    }
    Ok(d)
}

fn info_from_dict(d: &Bound<'_, PyDict>) -> PyResult<TerminalInfo> {
    fn req<'py, T: for<'a> FromPyObject<'a, 'py>>(d: &Bound<'py, PyDict>, k: &str) -> PyResult<T> {
        d.get_item(k)?
            .ok_or_else(|| PyValueError::new_err(format!("terminal info is missing {k:?}")))?
            .extract()
            .map_err(Into::into)
    }
    let colors = match d.get_item("colors")? {
        Some(c) if !c.is_none() => terminal_colors(Some(c.cast()?))?,
        _ => None,
    };
    Ok(TerminalInfo {
        cols: req(d, "cols")?,
        rows: req(d, "rows")?,
        cell_width: req(d, "cell_width")?,
        cell_height: req(d, "cell_height")?,
        kitty: req(d, "kitty")?,
        sixel: req(d, "sixel")?,
        tmux: req(d, "tmux")?,
        colors,
    })
}

/// Ask the controlling terminal for its size, graphics support and colors. Raises OSError when
/// there is no terminal.
#[pyfunction]
#[pyo3(signature = (timeout_ms=250))]
fn probe(py: Python<'_>, timeout_ms: u64) -> PyResult<Bound<'_, PyDict>> {
    let info = py
        .detach(|| show::probe(std::time::Duration::from_millis(timeout_ms)))
        .map_err(|e| PyOSError::new_err(e.to_string()))?;
    info_to_dict(py, &info)
}

/// The terminal size from the kernel (no queries): `(cols, rows, cell_width, cell_height)`,
/// with cell size `None` when the terminal doesn't report pixels.
#[pyfunction]
fn window_size() -> PyResult<(u16, u16, Option<u16>, Option<u16>)> {
    #[cfg(any(unix, windows))]
    {
        let tty = kittychart_term_tty()?;
        let ws = tty.window_size().map_err(|e| PyOSError::new_err(e.to_string()))?;
        let px = ws.cell_px();
        Ok((ws.cols, ws.rows, px.map(|p| p.0), px.map(|p| p.1)))
    }
    #[cfg(not(any(unix, windows)))]
    Err(PyOSError::new_err("no terminal on this platform"))
}

#[cfg(any(unix, windows))]
fn kittychart_term_tty() -> PyResult<kittychart::term::tty::Tty> {
    kittychart::term::tty::Tty::open().map_err(|e| PyOSError::new_err(e.to_string()))
}

#[derive(Debug, FromPyObject)]
#[pyo3(from_item_all)]
struct PyShowOptions {
    /// Required key (it may be None): pyo3 can't derive a struct whose fields all have defaults.
    protocol: Option<String>,
    #[pyo3(default)]
    cols: Option<u32>,
    #[pyo3(default)]
    rows: Option<u32>,
    #[pyo3(default)]
    width: Option<f32>,
    #[pyo3(default)]
    height: Option<f32>,
    #[pyo3(default)]
    scale: Option<f32>,
    #[pyo3(default)]
    background: Option<String>,
    #[pyo3(default)]
    placeholders: bool,
    #[pyo3(default)]
    force: bool,
    #[pyo3(default)]
    chartjs_colors: bool,
    #[pyo3(default)]
    image_id: Option<u32>,
}

impl PyShowOptions {
    fn to_show(&self) -> PyResult<ShowOptions> {
        Ok(ShowOptions {
            mode: mode(self.protocol.as_deref())?,
            cols: cells("cols", self.cols)?,
            rows: cells("rows", self.rows)?,
            width: positive("width", self.width)?,
            height: positive("height", self.height)?,
            scale: positive("scale", self.scale)?,
            background: color(self.background.as_deref())?,
            placeholders: self.placeholders,
            force: self.force,
            chartjs_colors: self.chartjs_colors,
            image_id: self.image_id.map(|i| i.clamp(1, 0x00FF_FFFF)),
        })
    }
}

/// A chart ready to write: what [`prepare`] and [`prepare_for`] return.
#[pyclass(frozen, get_all, module = "kittychart._native")]
#[derive(Debug)]
struct Prepared {
    /// "kitty", "sixel", "text", "blocks", "half" or "braille".
    mode: &'static str,
    /// Escapes or ANSI text, ending with a newline.
    data: Py<PyBytes>,
    /// "stdout" or "tty" (stdout is redirected, so graphics go to the terminal itself).
    target: &'static str,
    cols: u16,
    rows: u16,
    /// `data[:graphics_len]` are the graphics escapes alone.
    graphics_len: usize,
    image_id: Option<u32>,
}

fn to_py(py: Python<'_>, p: show::Prepared) -> Prepared {
    Prepared {
        mode: mode_name(p.mode),
        data: PyBytes::new(py, &p.bytes).unbind(),
        target: match p.target {
            show::Target::Stdout => "stdout",
            show::Target::Tty => "tty",
        },
        cols: p.cols,
        rows: p.rows,
        graphics_len: p.graphics_len,
        image_id: p.image_id,
    }
}

/// Detect the terminal, choose a mode and render, without writing.
#[pyfunction]
#[pyo3(signature = (json, opts, dispatch=None))]
fn prepare(
    py: Python<'_>,
    json: &str,
    opts: PyShowOptions,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<Prepared> {
    let (c, script) = config_with(json, dispatch)?;
    let o = opts.to_show()?;
    let p = py.detach(|| show::prepare(c, &o)).map_err(|e| match e {
        show::ShowError::Io(e) => PyOSError::new_err(e.to_string()),
        e => PyRuntimeError::new_err(e.to_string()),
    })?;
    check(script.as_ref())?;
    Ok(to_py(py, p))
}

/// Render for a known terminal (a dict from `probe`) without any I/O. `opts["protocol"]`
/// "auto" (or None) picks the best mode the terminal supports.
#[pyfunction]
#[pyo3(signature = (json, info, opts, dispatch=None))]
fn prepare_for(
    py: Python<'_>,
    json: &str,
    info: &Bound<'_, PyDict>,
    opts: PyShowOptions,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<Prepared> {
    let (c, script) = config_with(json, dispatch)?;
    let info = info_from_dict(info)?;
    let o = opts.to_show()?;
    let m = o.mode.unwrap_or(Mode::Auto);
    let p = py.detach(|| show::prepare_for(c, &info, m, &o));
    check(script.as_ref())?;
    Ok(to_py(py, p))
}

/// Render as text cells (truecolor ANSI). `colors` is a dict like `probe()["colors"]`.
#[pyfunction]
#[pyo3(signature = (json, cols, rows=None, mode="text", colors=None, dispatch=None))]
fn render_text(
    py: Python<'_>,
    json: &str,
    cols: u32,
    rows: Option<u32>,
    mode: &str,
    colors: Option<&Bound<'_, PyDict>>,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<String> {
    let cell = match mode {
        "text" => CellMode::Auto,
        "blocks" => CellMode::Blocks,
        "half" => CellMode::HalfBlocks,
        "braille" => CellMode::Braille,
        m => {
            return Err(PyValueError::new_err(format!(
                "unknown text mode {m:?}; expected text, blocks, half or braille"
            )));
        }
    };
    let cols = cells("cols", Some(cols))?.unwrap_or(80);
    let rows = cells("rows", rows)?;
    let colors = terminal_colors(colors)?;
    let (c, script) = config_with(json, dispatch)?;
    let r =
        py.detach(|| kittychart::terminal::render_cells_for_terminal(c, cols, rows, cell, colors.as_ref()));
    check(script.as_ref())?;
    String::from_utf8(r.bytes).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// Write bytes to the controlling terminal (`/dev/tty`).
#[pyfunction]
fn write_tty(py: Python<'_>, data: &[u8]) -> PyResult<()> {
    #[cfg(any(unix, windows))]
    {
        let mut tty = kittychart_term_tty()?;
        py.detach(|| tty.write_all(data)).map_err(|e| PyOSError::new_err(e.to_string()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (py, data);
        Err(PyOSError::new_err("no terminal on this platform"))
    }
}

/// Escapes that delete a kitty image (and its placements). Wrapped for tmux when `tmux`.
#[pyfunction]
#[pyo3(signature = (image_id, tmux=false))]
fn delete_image(py: Python<'_>, image_id: u32, tmux: bool) -> Bound<'_, PyBytes> {
    use kittychart::term::kitty;
    let mut out = Vec::new();
    kitty::delete_image(image_id, kitty::Quiet::Silent, &mut out);
    if tmux {
        out = kitty::tmux_passthrough(&out);
    }
    PyBytes::new(py, &out)
}

/// Rows of kitty Unicode placeholder text for an image (without colors: draw them with the
/// foreground color `placeholder_color(image_id)`).
#[pyfunction]
fn placeholder_rows(image_id: u32, cols: u32, rows: u32) -> Vec<String> {
    use kittychart::term::kitty;
    (0..rows).map(|r| (0..cols).map(|c| kitty::placeholder_cell(r, c, image_id)).collect()).collect()
}

/// The foreground color that carries `image_id` in placeholder cells.
#[pyfunction]
fn placeholder_color(image_id: u32) -> (u8, u8, u8) {
    kittychart::term::kitty::placeholder_fg(image_id)
}

/// The other id of a double-buffered kitty image (see `kitty::alternate_id`).
#[pyfunction]
fn alternate_id(image_id: u32) -> u32 {
    kittychart::term::kitty::alternate_id(image_id)
}

/// A fresh kitty image id.
#[pyfunction]
fn next_image_id() -> u32 {
    show::next_image_id()
}

fn cell_mode(mode: &str) -> PyResult<CellMode> {
    Ok(match mode {
        "text" => CellMode::Auto,
        "blocks" => CellMode::Blocks,
        "half" => CellMode::HalfBlocks,
        "braille" => CellMode::Braille,
        m => {
            return Err(PyValueError::new_err(format!(
                "unknown text mode {m:?}; expected text, blocks, half or braille"
            )));
        }
    })
}

/// A chart that lives across frames: hover, tooltip, legend clicks and animation (for TUI
/// widgets). Coordinates are CSS pixels of the chart (cells × cell size ÷ scale).
#[pyclass(unsendable, module = "kittychart._native")]
struct Live {
    chart: kittychart::Chart,
    script: Option<Script>,
    dispatch: Option<Py<PyAny>>,
    cells: bool,
    theme: Option<kittychart::Theme>,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live").field("cells", &self.cells).finish_non_exhaustive()
    }
}

#[pymethods]
impl Live {
    /// `cells`: drawn as text cells. `colors`: the surface's colors (like `probe()["colors"]`).
    #[new]
    #[pyo3(signature = (json, dispatch=None, cells=false, colors=None))]
    fn new(
        json: &str,
        dispatch: Option<Py<PyAny>>,
        cells: bool,
        colors: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Live> {
        let (c, script) = config_with(json, dispatch.as_ref().map(|d| Python::attach(|py| d.clone_ref(py))))?;
        let theme = terminal_colors(colors)?.as_ref().map(kittychart::Theme::from_terminal);
        let chart = if cells {
            kittychart::Chart::for_cells(c, theme.clone())
        } else {
            kittychart::Chart::with_theme(c, theme.clone())
        };
        Ok(Live { chart, script, dispatch, cells, theme })
    }

    /// New data or options: animate to them (when a clock is set).
    fn update(&mut self, json: &str) -> PyResult<()> {
        let d = self.dispatch.as_ref().map(|d| Python::attach(|py| d.clone_ref(py)));
        let (c, script) = config_with(json, d)?;
        self.script = script;
        self.chart.update(c);
        Ok(())
    }

    /// Hover at `(x, y)` (or nowhere, with `None`): returns whether anything changed.
    #[pyo3(signature = (x=None, y=None))]
    fn hover(&mut self, x: Option<f64>, y: Option<f64>) -> bool {
        let p = x.zip(y);
        let (active, tooltip) = match p {
            Some((x, y)) => (
                self.chart.elements_at(x, y, &self.chart.hover_interaction()),
                self.chart.elements_at(x, y, &self.chart.tooltip_interaction()),
            ),
            None => (vec![], vec![]),
        };
        let changed = active.as_slice() != self.chart.active() || !tooltip.is_empty() || p.is_none();
        self.chart.set_active(active);
        self.chart.set_tooltip(tooltip, p);
        changed
    }

    /// A click: toggles the legend item at `(x, y)`. Returns whether it did.
    fn click(&mut self, x: f64, y: f64) -> bool {
        self.chart.legend_click(x, y)
    }

    /// Drive animations with this clock (milliseconds).
    fn set_time(&mut self, ms: f64) {
        self.chart.set_time(ms);
    }

    #[getter]
    fn animating(&self) -> bool {
        self.chart.is_animating()
    }

    /// The active elements as `[(dataset, index), …]`.
    #[getter]
    fn active(&self) -> Vec<(usize, usize)> {
        self.chart.active().iter().map(|a| (a.dataset, a.index)).collect()
    }

    /// Draw to RGBA at exactly `width × height` device pixels.
    fn render_rgba<'py>(
        &mut self,
        py: Python<'py>,
        width: u32,
        height: u32,
        scale: f32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut c = kittychart::canvas::SkiaCanvas::with_pixel_size(width.max(1), height.max(1), scale);
        self.chart.dpr = f64::from(scale);
        self.chart.draw(&mut c);
        check(self.script.as_ref())?;
        Ok(PyBytes::new(py, &c.to_rgba()))
    }

    /// Draw and encode as a kitty image for a `cols × rows` cell area: a virtual placement
    /// (for Unicode placeholders) when `virtual`, else displayed at the cursor.
    #[pyo3(signature = (cols, rows, cell_width, cell_height, image_id, tmux=false, virtual_=true))]
    #[allow(clippy::too_many_arguments)]
    fn render_kitty<'py>(
        &mut self,
        py: Python<'py>,
        cols: u32,
        rows: u32,
        cell_width: f32,
        cell_height: f32,
        image_id: u32,
        tmux: bool,
        virtual_: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        use kittychart::term::kitty;
        let scale = kittychart::terminal::default_scale(cell_height);
        let (w, h) = ((cols as f32 * cell_width).round() as u32, (rows as f32 * cell_height).round() as u32);
        let mut c = kittychart::canvas::SkiaCanvas::with_pixel_size(w.max(1), h.max(1), scale);
        self.chart.dpr = f64::from(scale);
        self.chart.draw(&mut c);
        check(self.script.as_ref())?;
        let rgba = c.to_rgba();
        let img = kittychart::term::RgbaImage::new(c.pixel_width(), c.pixel_height(), &rgba);
        // Any non-zero id: placeholders carry the top byte (double buffering uses it).
        let mut t = kitty::TransmitOptions::new(image_id.max(1));
        t.columns = Some(cols);
        t.rows = Some(rows);
        t.placement_id = Some(1);
        let mut out = Vec::new();
        if virtual_ {
            kitty::transmit_virtual(img, &t, &mut out);
        } else {
            kitty::transmit_and_display(img, &t, &mut out);
        }
        if tmux {
            out = kitty::tmux_passthrough(&out);
        }
        Ok(PyBytes::new(py, &out))
    }

    /// Draw as `cols × rows` text cells (truecolor ANSI, rows separated by `\r\n`).
    #[pyo3(signature = (cols, rows, mode="text"))]
    fn render_text(&mut self, cols: u32, rows: u32, mode: &str) -> PyResult<String> {
        let mut c = kittychart::canvas::cells::CellCanvas::new(cols.max(1), rows.max(1), cell_mode(mode)?);
        self.chart.draw(&mut c);
        check(self.script.as_ref())?;
        Ok(c.into_grid().to_ansi())
    }

    #[getter]
    fn cells(&self) -> bool {
        let _ = &self.theme;
        self.cells
    }
}

/// Run the full-screen interactive viewer until the user quits.
#[pyfunction]
#[pyo3(signature = (json, opts, dispatch=None))]
fn interact(
    py: Python<'_>,
    json: &str,
    opts: &Bound<'_, PyDict>,
    dispatch: Option<Py<PyAny>>,
) -> PyResult<()> {
    #[cfg(any(unix, windows))]
    {
        let (c, script) = config_with(json, dispatch)?;
        let get = |k: &str| -> PyResult<Option<Bound<'_, PyAny>>> {
            Ok(opts.get_item(k)?.filter(|v| !v.is_none()))
        };
        let mode = get("protocol")?.map(|v| v.extract::<String>()).transpose()?;
        let o = kittychart::interact::InteractOptions {
            mode: mode.as_deref().map(str::parse::<Mode>).transpose().map_err(PyValueError::new_err)?,
            scale: get("scale")?.map(|v| v.extract()).transpose()?,
            background: color(get("background")?.map(|v| v.extract::<String>()).transpose()?.as_deref())?,
            chartjs_colors: get("chartjs_colors")?.map(|v| v.extract()).transpose()?.unwrap_or(false),
            no_fast_transfer: get("no_fast_transfer")?.map(|v| v.extract()).transpose()?.unwrap_or(false),
            watch: get("watch")?.map(|v| v.extract::<std::path::PathBuf>()).transpose()?,
            save_dir: None,
        };
        let script2 = script.clone();
        let r = py.detach(move || {
            kittychart::interact::interact(c, &o, |json| {
                let c = ChartConfig::from_json(json).map_err(|e| e.to_string())?;
                Ok(match &script2 {
                    Some(s) => c.with_script(s.clone()),
                    None => c,
                })
            })
        });
        check(script.as_ref())?;
        r.map_err(|e| match e {
            show::ShowError::Io(e) => PyOSError::new_err(e.to_string()),
            e => PyRuntimeError::new_err(e.to_string()),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (py, json, opts, dispatch);
        Err(PyOSError::new_err("interactive charts need a Unix or Windows terminal"))
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Live>()?;
    m.add_function(wrap_pyfunction!(interact, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Prepared>()?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add_function(wrap_pyfunction!(render_png, m)?)?;
    m.add_function(wrap_pyfunction!(render_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(render_svg, m)?)?;
    m.add_function(wrap_pyfunction!(render_text, m)?)?;
    m.add_function(wrap_pyfunction!(probe, m)?)?;
    m.add_function(wrap_pyfunction!(window_size, m)?)?;
    m.add_function(wrap_pyfunction!(prepare, m)?)?;
    m.add_function(wrap_pyfunction!(prepare_for, m)?)?;
    m.add_function(wrap_pyfunction!(write_tty, m)?)?;
    m.add_function(wrap_pyfunction!(delete_image, m)?)?;
    m.add_function(wrap_pyfunction!(placeholder_rows, m)?)?;
    m.add_function(wrap_pyfunction!(placeholder_color, m)?)?;
    m.add_function(wrap_pyfunction!(next_image_id, m)?)?;
    m.add_function(wrap_pyfunction!(alternate_id, m)?)?;
    Ok(())
}
