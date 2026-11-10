//! Chart.js-style option resolution over JSON scopes.
//!
//! Chart.js resolves every option by searching an ordered list of scopes (dataset config, chart
//! options, per-type overrides, global defaults) and returning the first scope that defines the
//! key. A nested object (like `font`) resolves key by key across the same scopes, so a dataset can
//! override `font.size` and still inherit `font.family`. [`Scopes`] implements that model over
//! `serde_json::Value`. Chart code converts the results into typed structs once per update.

use std::fmt;
use std::rc::Rc;

use elsa::FrozenVec;
use kittychart_canvas::Color;
use serde_json::Value;

use crate::color;
use crate::script::{self, Script};

/// The context scriptable options are called with (see [`crate::script`]): a base object (the
/// chart, dataset or scale context) plus, for indexable options, fields that depend on the
/// element index (`dataIndex`, `raw`, `parsed` … or `tick`).
pub struct ScriptCtx<'a> {
    pub script: Script,
    pub base: Value,
    pub per_index: Option<Box<dyn Fn(usize) -> Value + 'a>>,
}

impl fmt::Debug for ScriptCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptCtx")
            .field("base", &self.base)
            .field("per_index", &self.per_index.is_some())
            .finish()
    }
}

impl ScriptCtx<'_> {
    /// The context object for an option read at `index`.
    pub fn context(&self, index: Option<usize>) -> Value {
        let mut ctx = self.base.clone();
        if let (Some(i), Some(f), Value::Object(o)) = (index, &self.per_index, &mut ctx)
            && let Value::Object(extra) = f(i)
        {
            o.extend(extra);
        }
        ctx
    }
}

/// A scope object: borrowed from the configuration, or produced by a scriptable function.
#[derive(Clone)]
enum Src<'a> {
    Borrowed(&'a Value),
    Owned(Rc<Value>),
}

impl Src<'_> {
    fn value(&self) -> &Value {
        match self {
            Src::Borrowed(v) => v,
            Src::Owned(v) => v,
        }
    }
}

/// An ordered list of JSON objects searched first to last.
///
/// Values come back borrowed from the `Scopes` (not the configuration), because a scriptable
/// option's value is computed on demand; computed values live as long as the `Scopes`.
pub struct Scopes<'a> {
    scopes: Vec<Src<'a>>,
    /// Option-name prefixes tried in order, such as `["point", ""]` so a line dataset's
    /// `pointRadius` feeds the point element's `radius`.
    prefixes: Vec<String>,
    ctx: Option<Rc<ScriptCtx<'a>>>,
    computed: FrozenVec<Box<Value>>,
}

impl Clone for Scopes<'_> {
    fn clone(&self) -> Self {
        Scopes {
            scopes: self.scopes.clone(),
            prefixes: self.prefixes.clone(),
            ctx: self.ctx.clone(),
            computed: FrozenVec::new(),
        }
    }
}

impl Default for Scopes<'_> {
    fn default() -> Self {
        Scopes { scopes: Vec::new(), prefixes: vec![String::new()], ctx: None, computed: FrozenVec::new() }
    }
}

impl fmt::Debug for Scopes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scopes")
            .field("scopes", &self.scopes.iter().map(Src::value).collect::<Vec<_>>())
            .field("prefixes", &self.prefixes)
            .field("scriptable", &self.ctx.is_some())
            .finish()
    }
}

impl<'a> Scopes<'a> {
    pub fn new(scopes: impl IntoIterator<Item = &'a Value>) -> Self {
        Scopes {
            scopes: scopes.into_iter().filter(|v| v.is_object()).map(Src::Borrowed).collect(),
            ..Scopes::default()
        }
    }

    pub fn with_prefixes(mut self, prefixes: &[&str]) -> Self {
        self.prefixes = prefixes.iter().map(|p| (*p).to_owned()).collect();
        self
    }

    /// Resolve function markers through `ctx` (see [`crate::script`]).
    pub fn with_script(mut self, ctx: Option<Rc<ScriptCtx<'a>>>) -> Self {
        self.ctx = ctx;
        self
    }

    /// Append lower-priority scopes, for example a route such as `{"color": <chart color>}`.
    pub fn then(mut self, more: impl IntoIterator<Item = &'a Value>) -> Self {
        self.scopes.extend(more.into_iter().filter(|v| v.is_object()).map(Src::Borrowed));
        self
    }

    /// Append another resolver's scopes (lower priority).
    pub fn then_scopes(mut self, more: &Scopes<'a>) -> Self {
        self.scopes.extend(more.scopes.iter().cloned());
        self
    }

    /// Call the function behind a marker; `None` when it isn't a marker's or yields undefined.
    fn call(&self, id: &str, index: Option<usize>) -> Option<Value> {
        let ctx = self.ctx.as_ref()?;
        ctx.script.call(id, &ctx.context(index))
    }

    /// Resolve `key` (with prefixes) to the first defined value, calling functions with
    /// `index` in their context. JSON `null` counts as defined, as in Chart.js, where only
    /// `undefined` falls through; a function returning undefined falls through.
    fn find(&self, key: &str, index: Option<usize>) -> Option<&Value> {
        for prefix in &self.prefixes {
            let k = read_key(prefix, key);
            for s in &self.scopes {
                let Some(v) = s.value().get(&k) else { continue };
                match script::fn_id(v) {
                    None => return Some(v),
                    Some(id) => {
                        if let Some(r) = self.call(&id, index) {
                            return Some(self.computed.push_get(Box::new(r)));
                        }
                    }
                }
            }
        }
        None
    }

    /// The function id behind `key`, if its first definition is a function marker (for
    /// callbacks with their own calling convention, such as `ticks.callback`).
    pub fn fn_id(&self, key: &str) -> Option<String> {
        self.ctx.as_ref()?;
        for prefix in &self.prefixes {
            let k = read_key(prefix, key);
            if let Some(v) = self.scopes.iter().find_map(|s| s.value().get(&k)) {
                return script::fn_id(v);
            }
        }
        None
    }

    /// The script these scopes resolve functions with.
    pub fn script(&self) -> Option<&Script> {
        self.ctx.as_ref().map(|c| &c.script)
    }

    /// Resolve a key: for each prefix, the first scope that defines `prefix + Key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.find(key, None)
    }

    /// The scopes of a nested object option, merged key by key (Chart.js sub-resolver).
    /// Prefixes don't carry into sub-objects. A function returning an object contributes it.
    pub fn child(&self, key: &str) -> Scopes<'a> {
        let mut scopes = Vec::new();
        for prefix in &self.prefixes {
            let k = read_key(prefix, key);
            for s in &self.scopes {
                let found = match s {
                    Src::Borrowed(v) => v.get(&k).map(Src::Borrowed),
                    Src::Owned(v) => v.get(&k).map(|c| Src::Owned(Rc::new(c.clone()))),
                };
                let Some(found) = found else { continue };
                match script::fn_id(found.value()) {
                    Some(id) => {
                        if let Some(r) = self.call(&id, None).filter(Value::is_object) {
                            scopes.push(Src::Owned(Rc::new(r)));
                        }
                    }
                    None if found.value().is_object() => scopes.push(found),
                    None => {}
                }
            }
        }
        Scopes { scopes, prefixes: vec![String::new()], ctx: self.ctx.clone(), computed: FrozenVec::new() }
    }

    /// Like [`Scopes::get`], with indexable-option semantics: an array value yields
    /// `array[index % len]`, and functions see `index` in their context.
    pub fn get_at(&self, key: &str, index: Option<usize>) -> Option<&Value> {
        let v = self.find(key, index)?;
        match (v, index) {
            (Value::Array(a), Some(i)) if !a.is_empty() => Some(&a[i % a.len()]),
            _ => Some(v),
        }
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(as_num)
    }
    pub fn num_at(&self, key: &str, i: Option<usize>) -> Option<f64> {
        self.get_at(key, i).and_then(as_num)
    }
    pub fn num_or(&self, key: &str, default: f64) -> f64 {
        self.num(key).unwrap_or(default)
    }
    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(default)
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }
    pub fn color_at(&self, key: &str, i: Option<usize>) -> Option<Color> {
        self.get_at(key, i).and_then(Value::as_str).and_then(color::parse)
    }
    /// Number arrays such as `borderDash` (not indexable).
    pub fn num_list(&self, key: &str) -> Vec<f64> {
        self.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(as_num).collect())
            .unwrap_or_default()
    }
    /// No scope objects at all (e.g. an unset nested option).
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
    pub fn is_defined(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| !v.is_null())
    }
}

fn read_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        return key.to_owned();
    }
    let mut c = key.chars();
    match c.next() {
        Some(f) => format!("{prefix}{}{}", f.to_ascii_uppercase(), c.as_str()),
        None => prefix.to_owned(),
    }
}

/// Numbers, plus numeric strings, as JavaScript coerces them in arithmetic.
pub fn as_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Deep `mergeIf`: copy keys from each source into `target` only where `target` lacks them,
/// recursing into objects. Earlier sources win.
pub fn merge_if(target: &mut Value, sources: &[&Value]) {
    for src in sources {
        let (Value::Object(t), Value::Object(s)) = (&mut *target, src) else { continue };
        for (k, sv) in s {
            match t.get_mut(k) {
                None => {
                    t.insert(k.clone(), sv.clone());
                }
                Some(tv) if tv.is_object() && sv.is_object() => merge_if(tv, &[sv]),
                Some(_) => {}
            }
        }
    }
}

/// Edge sizes, as `toPadding` / `toTRBL` produce them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Padding {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Padding {
    pub fn width(&self) -> f64 {
        self.left + self.right
    }
    pub fn height(&self) -> f64 {
        self.top + self.bottom
    }

    /// `toPadding` / `toTRBL`: a number applies to all sides, and an object reads
    /// `top/right/bottom/left`, falling back to `y`/`x`.
    pub fn from_value(v: Option<&Value>) -> Self {
        match v {
            Some(Value::Object(o)) => {
                let read =
                    |k: &str, alt: &str| o.get(k).or_else(|| o.get(alt)).and_then(as_num).unwrap_or(0.0);
                Padding {
                    top: read("top", "y"),
                    right: read("right", "x"),
                    bottom: read("bottom", "y"),
                    left: read("left", "x"),
                }
            }
            Some(v) => {
                let n = as_num(v).unwrap_or(0.0);
                Padding { top: n, right: n, bottom: n, left: n }
            }
            None => Padding::default(),
        }
    }
}

/// Corner radii (`toTRBLCorners`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Corners {
    pub top_left: f64,
    pub top_right: f64,
    pub bottom_left: f64,
    pub bottom_right: f64,
}

impl Corners {
    pub fn from_value(v: Option<&Value>) -> Self {
        match v {
            Some(Value::Object(o)) => {
                let r = |k: &str| o.get(k).and_then(as_num).unwrap_or(0.0);
                Corners {
                    top_left: r("topLeft"),
                    top_right: r("topRight"),
                    bottom_left: r("bottomLeft"),
                    bottom_right: r("bottomRight"),
                }
            }
            Some(v) => {
                let n = as_num(v).unwrap_or(0.0);
                Corners { top_left: n, top_right: n, bottom_left: n, bottom_right: n }
            }
            None => Corners::default(),
        }
    }
    pub fn any(&self) -> bool {
        self.top_left != 0.0 || self.top_right != 0.0 || self.bottom_left != 0.0 || self.bottom_right != 0.0
    }
}

/// A resolved `FontSpec` (`toFont`).
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub family: String,
    pub size: f64,
    pub style: String,
    pub weight: Option<u16>,
    pub line_height: f64,
}

impl FontSpec {
    /// Resolve a font from `font` option scopes, falling back key by key to `fallback`'s scopes
    /// (the chart and global `font`).
    pub fn resolve(font: &Scopes<'_>, fallback: &Scopes<'_>) -> FontSpec {
        let get = |k: &str| font.get(k).filter(|v| !v.is_null()).or_else(|| fallback.get(k));
        let size = get("size").and_then(as_num).unwrap_or(12.0);
        let family = get("family").and_then(Value::as_str).unwrap_or("sans-serif").to_owned();
        let style = get("style").and_then(Value::as_str).unwrap_or("normal").to_owned();
        let weight = get("weight").and_then(|w| match w {
            Value::String(s) => match s.as_str() {
                "bold" | "bolder" => Some(700),
                "normal" => Some(400),
                "lighter" => Some(300),
                other => other.parse().ok(),
            },
            Value::Number(n) => n.as_u64().map(|n| n as u16),
            _ => None,
        });
        let line_height = to_line_height(get("lineHeight"), size);
        FontSpec { family, size, style, weight, line_height }
    }

    pub fn to_canvas(&self) -> kittychart_canvas::Font {
        kittychart_canvas::Font {
            family: self.family.clone(),
            size: self.size as f32,
            weight: self.weight.unwrap_or(400),
            italic: self.style == "italic" || self.style.starts_with("oblique"),
        }
    }
}

/// `toLineHeight`: a number multiplies the size; strings may be `px`, `em` or `%`.
pub fn to_line_height(v: Option<&Value>, size: f64) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(1.2) * size,
        Some(Value::String(s)) => {
            let s = s.trim();
            if s == "normal" {
                return size * 1.2;
            }
            if let Some(px) = s.strip_suffix("px") {
                return px.parse().unwrap_or(size * 1.2);
            }
            if let Some(pc) = s.strip_suffix('%') {
                return pc.parse::<f64>().map_or(size * 1.2, |p| size * p / 100.0);
            }
            let num = s.strip_suffix("em").unwrap_or(s);
            num.parse::<f64>().map_or(size * 1.2, |m| size * m)
        }
        _ => size * 1.2,
    }
}

/// `toDimension`: a number, or a percentage string relative to `of`.
pub fn to_dimension(v: Option<&Value>, of: f64) -> f64 {
    match v {
        Some(Value::String(s)) if s.trim().ends_with('%') => {
            s.trim().trim_end_matches('%').parse::<f64>().map_or(0.0, |p| p / 100.0 * of)
        }
        Some(v) => as_num(v).unwrap_or(0.0),
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scopes_resolve_first_definition_and_merge_children() {
        let dataset = json!({"borderWidth": 5, "font": {"size": 20}});
        let defaults = json!({"borderWidth": 1, "radius": 3, "font": {"size": 12, "family": "Inter"}});
        let s = Scopes::new([&dataset, &defaults]);
        assert_eq!(s.num("borderWidth"), Some(5.0));
        assert_eq!(s.num("radius"), Some(3.0));
        let f = s.child("font");
        assert_eq!(f.num("size"), Some(20.0));
        assert_eq!(f.str("family"), Some("Inter"));
    }

    #[test]
    fn prefixes_and_indexable_arrays() {
        let dataset = json!({"pointRadius": [1, 2], "radius": 9, "backgroundColor": ["red", "#00f"]});
        let s = Scopes::new([&dataset]).with_prefixes(&["point", ""]);
        assert_eq!(s.num_at("radius", Some(3)), Some(2.0));
        assert_eq!(s.color_at("backgroundColor", Some(1)), crate::color::parse("blue"));
    }

    #[test]
    fn merge_if_keeps_existing() {
        let mut t = json!({"a": 1, "o": {"x": 1}});
        merge_if(&mut t, &[&json!({"a": 2, "b": 3, "o": {"x": 2, "y": 4}})]);
        assert_eq!(t, json!({"a": 1, "b": 3, "o": {"x": 1, "y": 4}}));
    }

    #[test]
    fn padding_and_fonts() {
        assert_eq!(
            Padding::from_value(Some(&json!({"x": 4, "top": 2}))),
            Padding { top: 2.0, right: 4.0, bottom: 0.0, left: 4.0 }
        );
        let font = json!({"weight": "bold"});
        let global = json!({"family": "Inter", "size": 12, "lineHeight": 1.2});
        let f = FontSpec::resolve(&Scopes::new([&font]), &Scopes::new([&global]));
        assert_eq!((f.size, f.weight, f.line_height), (12.0, Some(700), 12.0 * 1.2));
    }
}
