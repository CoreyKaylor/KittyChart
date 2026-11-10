//! The Chart.js configuration contract: `{ type, data: { labels, datasets }, options }`.

use serde_json::Value;

/// Errors from loading a configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("config must be an object with a \"type\" (e.g. \"line\" or \"bar\")")]
    MissingType,
    #[error("unsupported chart type {0:?} (supported so far: {SUPPORTED_TYPES})")]
    UnsupportedType(String),
    #[error("dataset {0}: \"data\" must be an array or an object")]
    BadData(usize),
}

const SUPPORTED_TYPES: &str = "line, bar, scatter, bubble, pie, doughnut, radar, polarArea";

/// Chart and dataset types with a controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartType {
    Line,
    Bar,
    Scatter,
    Bubble,
    Pie,
    Doughnut,
    Radar,
    PolarArea,
}

impl ChartType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "line" => Some(Self::Line),
            "bar" => Some(Self::Bar),
            "scatter" => Some(Self::Scatter),
            "bubble" => Some(Self::Bubble),
            "pie" => Some(Self::Pie),
            "doughnut" => Some(Self::Doughnut),
            "radar" => Some(Self::Radar),
            "polarArea" => Some(Self::PolarArea),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Bar => "bar",
            Self::Scatter => "scatter",
            Self::Bubble => "bubble",
            Self::Pie => "pie",
            Self::Doughnut => "doughnut",
            Self::Radar => "radar",
            Self::PolarArea => "polarArea",
        }
    }

    /// Controllers built on arc elements around a center, without cartesian scales.
    pub fn is_arc(self) -> bool {
        matches!(self, Self::Pie | Self::Doughnut)
    }

    /// Controllers on the radial linear scale.
    pub fn is_radial(self) -> bool {
        matches!(self, Self::Radar | Self::PolarArea)
    }

    /// Controllers on x/y cartesian scales.
    pub fn is_cartesian(self) -> bool {
        !self.is_arc() && !self.is_radial()
    }

    /// Controllers built on point elements (line, scatter, bubble).
    pub fn uses_points(self) -> bool {
        matches!(self, Self::Line | Self::Scatter | Self::Bubble)
    }
}

/// A Chart.js configuration. `data` and `options` stay as JSON so every Chart.js option, even
/// one this version ignores, round-trips and can be resolved through the scope chain.
#[derive(Debug, Clone)]
pub struct ChartConfig {
    pub chart_type: ChartType,
    pub data: Value,
    pub options: Value,
}

impl ChartConfig {
    pub fn from_json(s: &str) -> Result<Self, ConfigError> {
        Self::from_value(serde_json::from_str(s)?)
    }

    pub fn from_value(mut v: Value) -> Result<Self, ConfigError> {
        let obj = v.as_object_mut().ok_or(ConfigError::MissingType)?;
        let ty = obj.get("type").and_then(Value::as_str).ok_or(ConfigError::MissingType)?;
        let chart_type = ChartType::parse(ty).ok_or_else(|| ConfigError::UnsupportedType(ty.to_owned()))?;
        let mut data = obj.remove("data").unwrap_or_else(|| serde_json::json!({}));
        let options = obj.remove("options").filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
        if !data.is_object() {
            data = serde_json::json!({});
        }
        let d = data.as_object_mut().expect("checked object");
        d.entry("labels").or_insert_with(|| Value::Array(vec![]));
        let datasets = d.entry("datasets").or_insert_with(|| Value::Array(vec![]));
        if !datasets.is_array() {
            *datasets = Value::Array(vec![]);
        }
        for (i, ds) in datasets.as_array_mut().expect("array").iter_mut().enumerate() {
            if !ds.is_object() {
                *ds = serde_json::json!({});
            }
            let dso = ds.as_object_mut().expect("object");
            if let Some(t) = dso.get("type").and_then(Value::as_str)
                && ChartType::parse(t).is_none()
            {
                return Err(ConfigError::UnsupportedType(t.to_owned()));
            }
            match dso.get("data") {
                None | Some(Value::Null) => {
                    dso.insert("data".into(), Value::Array(vec![]));
                }
                Some(Value::Array(_) | Value::Object(_)) => {}
                Some(_) => return Err(ConfigError::BadData(i)),
            }
        }
        Ok(Self { chart_type, data, options })
    }

    pub fn datasets(&self) -> &[Value] {
        self.data["datasets"].as_array().map_or(&[], Vec::as_slice)
    }

    pub fn labels(&self) -> Vec<Label> {
        self.data["labels"].as_array().map(|a| a.iter().map(Label::from_value).collect()).unwrap_or_default()
    }
}

/// A tick or category label: one line, or several (`["Line 1", "Line 2"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum Label {
    Text(String),
    Lines(Vec<String>),
}

impl Label {
    pub fn from_value(v: &Value) -> Label {
        match v {
            Value::Array(a) => Label::Lines(a.iter().map(js_to_string).collect()),
            Value::Null => Label::Text(String::new()),
            other => Label::Text(js_to_string(other)),
        }
    }
    pub fn lines(&self) -> Vec<&str> {
        match self {
            Label::Text(s) => vec![s.as_str()],
            Label::Lines(l) => l.iter().map(String::as_str).collect(),
        }
    }
    pub fn line_count(&self) -> usize {
        match self {
            Label::Text(_) => 1,
            Label::Lines(l) => l.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.lines().iter().all(|l| l.is_empty())
    }
}

impl From<&str> for Label {
    fn from(s: &str) -> Self {
        Label::Text(s.to_owned())
    }
}

/// JavaScript `'' + value`.
pub fn js_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), js_number_to_string),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        Value::Array(a) => a.iter().map(js_to_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JavaScript `Number.prototype.toString()`: shortest round-trip digits, with exponent form
/// outside `1e-7 ≤ |x| < 1e21`.
pub fn js_number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    let a = x.abs();
    if (1e-7..1e21).contains(&a) {
        // Rust's `{}` prints the shortest round-trip decimal without exponent, as JS does here.
        format!("{x}")
    } else {
        let s = format!("{x:e}"); // e.g. "1.5e21", "-2e-8"
        let (m, e) = s.split_once('e').expect("exponent form");
        let e: i32 = e.parse().expect("exponent");
        format!("{m}e{}{}", if e >= 0 { "+" } else { "-" }, e.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_numbers() {
        assert_eq!(js_number_to_string(5.0), "5");
        assert_eq!(js_number_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(js_number_to_string(1e21), "1e+21");
        assert_eq!(js_number_to_string(-2.5e-8), "-2.5e-8");
        assert_eq!(js_number_to_string(123456789012.0), "123456789012");
    }

    #[test]
    fn normalizes_and_rejects() {
        let c = ChartConfig::from_json(r#"{"type":"bar","data":{"datasets":[{"data":null}]}}"#).unwrap();
        assert_eq!(c.datasets()[0]["data"], serde_json::json!([]));
        assert!(matches!(
            ChartConfig::from_json(r#"{"type":"funnel"}"#),
            Err(ConfigError::UnsupportedType(_))
        ));
        assert!(matches!(ChartConfig::from_json(r#"{"data":{}}"#), Err(ConfigError::MissingType)));
    }
}
