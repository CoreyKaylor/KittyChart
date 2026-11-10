//! Scriptable options: functions in place of option values, as in Chart.js.
//!
//! JSON can't hold functions, so a function is written as a marker object
//! `{"__kc_fn": "<id>"}` wherever Chart.js accepts a scriptable option. When the option is
//! resolved, the chart's [`Script`] is called with the id and a context object, and its result
//! is used as the value. Language bindings put the markers in for their own callables (Python
//! functions, JavaScript functions, C function pointers) and dispatch the calls back; Rust code
//! registers closures with [`crate::ChartConfig::with_fn`].
//!
//! The context mirrors Chart.js's option contexts (`type` is `chart`, `dataset`, `data`,
//! `scale`, `tick` or `segment`; plus `datasetIndex`, `dataIndex`, `raw`, `parsed`, `chart`
//! with its `chartArea` …). Callbacks that Chart.js calls with positional arguments, such as
//! `ticks.callback(value, index, ticks)`, get them as `__args` in the context; bindings spread
//! them into the call.
//!
//! A function that returns `null` (JavaScript `undefined`, Python `None`) leaves the option
//! undefined, so it falls through to the next scope. Errors don't abort drawing; they are
//! collected and reported by [`Script::take_errors`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// The key of a function marker.
pub const FN_KEY: &str = "__kc_fn";

/// A callback: `(function id, context) → value`.
pub type Dispatch = dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync;

/// A named Rust callback for [`crate::ChartConfig::with_fn`].
pub type NamedFn = Arc<dyn Fn(&Value) -> Result<Value, String> + Send + Sync>;

/// Calls scriptable-option functions. Cheap to clone (shared).
#[derive(Clone)]
pub struct Script {
    inner: Arc<Inner>,
}

struct Inner {
    named: BTreeMap<String, NamedFn>,
    dispatch: Option<Arc<Dispatch>>,
    errors: Mutex<Vec<String>>,
}

impl fmt::Debug for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Script")
            .field("named", &self.inner.named.keys().collect::<Vec<_>>())
            .field("dispatch", &self.inner.dispatch.is_some())
            .finish()
    }
}

impl Default for Script {
    fn default() -> Self {
        Script::from_parts(BTreeMap::new(), None)
    }
}

impl Script {
    fn from_parts(named: BTreeMap<String, NamedFn>, dispatch: Option<Arc<Dispatch>>) -> Script {
        Script { inner: Arc::new(Inner { named, dispatch, errors: Mutex::new(Vec::new()) }) }
    }

    /// A script that sends every call to `dispatch` (how bindings hook in).
    pub fn new(dispatch: impl Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static) -> Script {
        Script::from_parts(BTreeMap::new(), Some(Arc::new(dispatch)))
    }

    /// This script plus a named Rust function (named functions are tried before the dispatcher).
    pub fn with_named(&self, id: &str, f: NamedFn) -> Script {
        let mut named = self.inner.named.clone();
        named.insert(id.to_owned(), f);
        Script::from_parts(named, self.inner.dispatch.clone())
    }

    /// Call function `id`. `None` (undefined) on error, with the error recorded.
    pub fn call(&self, id: &str, ctx: &Value) -> Option<Value> {
        let r = match self.inner.named.get(id) {
            Some(f) => f(ctx),
            None => match &self.inner.dispatch {
                Some(d) => d(id, &js_numbers(ctx)),
                None => Err("no such function".into()),
            },
        };
        match r {
            Ok(Value::Null) => None,
            Ok(v) => Some(v),
            Err(e) => {
                let mut errors = self.inner.errors.lock().unwrap_or_else(|p| p.into_inner());
                if errors.len() < 100 {
                    errors.push(format!("function {id:?}: {e}"));
                }
                None
            }
        }
    }

    /// Errors from callbacks since the last call (each distinct message once).
    pub fn take_errors(&self) -> Vec<String> {
        let mut errors = std::mem::take(&mut *self.inner.errors.lock().unwrap_or_else(|p| p.into_inner()));
        errors.dedup();
        errors
    }
}

/// The function id, if `v` is a function marker.
pub fn fn_id(v: &Value) -> Option<String> {
    match v.as_object()?.get(FN_KEY)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Whether `v` contains a function marker anywhere.
pub fn has_fns(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.contains_key(FN_KEY) || o.values().any(has_fns),
        Value::Array(a) => a.iter().any(has_fns),
        _ => false,
    }
}

/// A function marker value.
pub fn marker(id: &str) -> Value {
    serde_json::json!({ FN_KEY: id })
}

/// JavaScript has one number type, so a whole number crosses into another language as an
/// integer: Python's `f"${value}"` then prints `$15`, as Chart.js does, not `$15.0`.
fn js_numbers(v: &Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 => {
                Value::from(f as i64)
            }
            _ => v.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(js_numbers).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), js_numbers(v))).collect()),
        _ => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn whole_numbers_reach_other_languages_as_integers() {
        let seen = Arc::new(Mutex::new(Value::Null));
        let s = seen.clone();
        let script = Script::new(move |_, ctx| {
            *s.lock().unwrap() = ctx.clone();
            Ok(Value::Null)
        });
        script.call("f", &json!({"__args": [15.0, 2, [{"value": -3.0}, {"value": 2.5}]], "big": 1e300}));
        assert_eq!(
            seen.lock().unwrap().to_string(),
            r#"{"__args":[15,2,[{"value":-3},{"value":2.5}]],"big":1e+300}"#
        );
    }
}
