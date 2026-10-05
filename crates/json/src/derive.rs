//! Conversions for primitive types plus `json_struct!` / `json_enum!` macros
//! that generate strict `ToJson` / `FromJson` implementations.
//!
//! `FromJson` implementations reject non-finite numbers, wrong types and
//! out-of-range integers, so every decoded IPC payload or stored document is
//! validated structurally before domain validation runs.

use crate::{err, FromJson, JResult, JsonError, ToJson, Value};
use std::collections::BTreeMap;

macro_rules! num_json {
    ($($t:ty),*) => {$(
        impl ToJson for $t {
            fn to_json(&self) -> Value { Value::from(*self) }
        }
    )*};
}
num_json!(f64, f32, i8, i16, i32, i64, u8, u16, u32, u64, usize);

impl FromJson for f64 {
    fn from_json(v: &Value) -> JResult<Self> {
        match v {
            Value::Num(n) if n.is_finite() => Ok(*n),
            _ => err("expected a finite number"),
        }
    }
}
impl FromJson for f32 {
    fn from_json(v: &Value) -> JResult<Self> {
        let n = f64::from_json(v)?;
        if n.abs() > f32::MAX as f64 {
            return err("number out of range");
        }
        Ok(n as f32)
    }
}

macro_rules! int_from_json {
    ($($t:ty),*) => {$(
        impl FromJson for $t {
            fn from_json(v: &Value) -> JResult<Self> {
                match v.as_i64() {
                    Some(n) if n >= <$t>::MIN as i64 && (n as i128) <= <$t>::MAX as i128 => Ok(n as $t),
                    Some(_) => err(concat!("integer out of range for ", stringify!($t))),
                    None => err("expected an integer"),
                }
            }
        }
    )*};
}
int_from_json!(i8, i16, i32, i64, u8, u16, u32, u64, usize);

impl ToJson for bool {
    fn to_json(&self) -> Value {
        Value::Bool(*self)
    }
}
impl FromJson for bool {
    fn from_json(v: &Value) -> JResult<Self> {
        v.as_bool().ok_or_else(|| JsonError("expected a boolean".into()))
    }
}
impl ToJson for String {
    fn to_json(&self) -> Value {
        Value::Str(self.clone())
    }
}
impl FromJson for String {
    fn from_json(v: &Value) -> JResult<Self> {
        v.as_str().map(|s| s.to_string()).ok_or_else(|| JsonError("expected a string".into()))
    }
}
impl ToJson for Value {
    fn to_json(&self) -> Value {
        self.clone()
    }
}
impl FromJson for Value {
    fn from_json(v: &Value) -> JResult<Self> {
        Ok(v.clone())
    }
}
impl<T: FromJson> FromJson for Option<T> {
    fn from_json(v: &Value) -> JResult<Self> {
        if v.is_null() {
            Ok(None)
        } else {
            T::from_json(v).map(Some)
        }
    }
}
impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(v: &Value) -> JResult<Self> {
        match v {
            Value::Arr(a) => a
                .iter()
                .enumerate()
                .map(|(i, x)| T::from_json(x).map_err(|e| JsonError(format!("[{i}]: {}", e.0))))
                .collect(),
            _ => err("expected an array"),
        }
    }
}
impl<T: ToJson> ToJson for BTreeMap<String, T> {
    fn to_json(&self) -> Value {
        Value::Obj(self.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
    }
}
impl<T: FromJson> FromJson for BTreeMap<String, T> {
    fn from_json(v: &Value) -> JResult<Self> {
        match v {
            Value::Obj(o) => o
                .iter()
                .map(|(k, x)| T::from_json(x).map(|t| (k.clone(), t)).map_err(|e| JsonError(format!("{k}: {}", e.0))))
                .collect(),
            _ => err("expected an object"),
        }
    }
}
impl<A: ToJson, B: ToJson> ToJson for (A, B) {
    fn to_json(&self) -> Value {
        Value::Arr(vec![self.0.to_json(), self.1.to_json()])
    }
}
impl<A: FromJson, B: FromJson> FromJson for (A, B) {
    fn from_json(v: &Value) -> JResult<Self> {
        match v.as_arr() {
            Some(a) if a.len() == 2 => Ok((A::from_json(&a[0])?, B::from_json(&a[1])?)),
            _ => err("expected a 2-element array"),
        }
    }
}

static NULL: Value = Value::Null;

/// Read a field, attaching the key name to any error. Missing keys are
/// treated as null (so `Option` fields may be omitted).
pub fn field<T: FromJson>(obj: &Value, key: &str) -> JResult<T> {
    if !matches!(obj, Value::Obj(_)) {
        return err(format!("expected an object while reading '{key}'"));
    }
    let v = obj.get(key).unwrap_or(&NULL);
    T::from_json(v).map_err(|e| {
        if v.is_null() {
            JsonError(format!("missing field '{key}'"))
        } else {
            JsonError(format!("{key}: {}", e.0))
        }
    })
}

/// Like [`field`] but returns `default()` when the key is missing or null.
pub fn field_or<T: FromJson>(obj: &Value, key: &str, default: impl FnOnce() -> T) -> JResult<T> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default()),
        Some(v) => T::from_json(v).map_err(|e| JsonError(format!("{key}: {}", e.0))),
    }
}

/// Generate `ToJson`/`FromJson` for a struct with named fields.
///
/// ```ignore
/// json_struct!(Foo { a: "a", b: "bKey" = 5 });
/// ```
/// A field with `= expr` uses that default when absent (useful for
/// forward-compatible document migrations).
#[macro_export]
macro_rules! json_struct {
    ($name:ident { $($field:ident : $key:literal $(= $def:expr)?),* $(,)? }) => {
        impl $crate::ToJson for $name {
            fn to_json(&self) -> $crate::Value {
                let mut v = $crate::Value::empty_obj();
                $( v.set($key, $crate::ToJson::to_json(&self.$field)); )*
                v
            }
        }
        impl $crate::FromJson for $name {
            fn from_json(j: &$crate::Value) -> $crate::JResult<Self> {
                Ok($name { $( $field: $crate::__json_get!(j, $key $(, $def)?), )* })
            }
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __json_get {
    ($j:expr, $key:literal) => {
        $crate::field($j, $key)?
    };
    ($j:expr, $key:literal, $def:expr) => {
        $crate::field_or($j, $key, || $def)?
    };
}

/// Generate string-backed `ToJson`/`FromJson` for a fieldless enum.
#[macro_export]
macro_rules! json_enum {
    ($name:ident { $($var:ident = $s:literal),* $(,)? }) => {
        #[allow(dead_code)]
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $( $name::$var => $s, )* }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $( $s => Some($name::$var), )* _ => None }
            }
            pub fn all() -> &'static [$name] {
                &[$( $name::$var, )*]
            }
        }
        impl $crate::ToJson for $name {
            fn to_json(&self) -> $crate::Value { $crate::Value::Str(self.as_str().to_string()) }
        }
        impl $crate::FromJson for $name {
            fn from_json(v: &$crate::Value) -> $crate::JResult<Self> {
                match v.as_str() {
                    Some(s) => $name::parse(s).ok_or_else(|| $crate::JsonError(format!(
                        "unknown {} value '{}'", stringify!($name), s))),
                    None => Err($crate::JsonError(format!("{} must be a string", stringify!($name)))),
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Color {
        Red,
        Blue,
    }
    json_enum!(Color { Red = "red", Blue = "blue" });

    #[derive(Debug, Clone, PartialEq)]
    struct Thing {
        n: u16,
        name: String,
        opt: Option<f64>,
        list: Vec<Color>,
        dflt: u32,
    }
    json_struct!(Thing { n: "n", name: "name", opt: "opt", list: "list", dflt: "dflt" = 7 });

    #[test]
    fn struct_roundtrip_and_defaults() {
        let t = Thing { n: 3, name: "x".into(), opt: None, list: vec![Color::Red, Color::Blue], dflt: 9 };
        let v = t.to_json();
        assert_eq!(Thing::from_json(&v).unwrap(), t);
        let v = parse(r#"{"n":1,"name":"a","list":["blue"]}"#).unwrap();
        let t = Thing::from_json(&v).unwrap();
        assert_eq!(t.dflt, 7);
        assert_eq!(t.opt, None);
    }

    #[test]
    fn strict_errors() {
        let e = Thing::from_json(&parse(r#"{"n":70000,"name":"a","list":[]}"#).unwrap()).unwrap_err();
        assert!(e.0.contains("n:"), "{}", e.0);
        let e = Thing::from_json(&parse(r#"{"n":1,"list":[]}"#).unwrap()).unwrap_err();
        assert!(e.0.contains("missing field 'name'"), "{}", e.0);
        let e = Thing::from_json(&parse(r#"{"n":1,"name":"a","list":["green"]}"#).unwrap()).unwrap_err();
        assert!(e.0.contains("green"), "{}", e.0);
        assert!(u8::from_json(&Value::Num(1.5)).is_err());
        assert!(u8::from_json(&Value::Num(-1.0)).is_err());
    }
}
