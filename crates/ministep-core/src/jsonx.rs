//! Lenient JSON accessors mirroring Python's `int(x)` / `float(x)` / `bool(x)` coercions
//! used by the original loaders, so files written by any MiniStep build still load.

use serde_json::Value;

pub fn as_f64(v: &Value, what: &str) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("{what} must be a number")),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::String(s) => s.trim().parse().map_err(|_| format!("could not convert string to float: {s:?}")),
        _ => Err(format!("{what} must be a number")),
    }
}

pub fn as_i64(v: &Value, what: &str) -> Result<i64, String> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.trunc() as i64))
            .ok_or_else(|| format!("{what} must be an integer")),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::String(s) => s.trim().parse().map_err(|_| format!("invalid literal for int(): {s:?}")),
        _ => Err(format!("{what} must be an integer")),
    }
}

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn get_f64(obj: &serde_json::Map<String, Value>, key: &str, default: f64) -> Result<f64, String> {
    obj.get(key).map_or(Ok(default), |v| as_f64(v, key))
}

pub fn get_i64(obj: &serde_json::Map<String, Value>, key: &str, default: i64) -> Result<i64, String> {
    obj.get(key).map_or(Ok(default), |v| as_i64(v, key))
}
