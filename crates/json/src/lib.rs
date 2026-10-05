//! Minimal JSON support used across Ridgeline.
//!
//! Design goals: strict validation of untrusted input (IPC payloads, AI output,
//! imported files), deterministic output ordering, and no third-party
//! dependencies so the core can be built and tested offline.
//!
//! Non-finite numbers are rejected on both parse and serialize.

use std::fmt;

mod derive;
pub use derive::*;

pub const MAX_DEPTH: usize = 64;
pub const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct JsonError(pub String);

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for JsonError {}

pub type JResult<T> = Result<T, JsonError>;

pub fn err<T>(msg: impl Into<String>) -> JResult<T> {
    Err(JsonError(msg.into()))
}

// ---------------------------------------------------------------- building

impl Value {
    pub fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Value {
        Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn empty_obj() -> Value {
        Value::Obj(Vec::new())
    }
    /// Insert or replace a key on an object. No-op on non-objects.
    pub fn set(&mut self, key: &str, v: impl Into<Value>) {
        if let Value::Obj(pairs) = self {
            let v = v.into();
            if let Some(p) = pairs.iter_mut().find(|(k, _)| k == key) {
                p.1 = v;
            } else {
                pairs.push((key.to_string(), v));
            }
        }
    }
    pub fn with(mut self, key: &str, v: impl Into<Value>) -> Value {
        self.set(key, v);
        self
    }
    pub fn push(&mut self, v: impl Into<Value>) {
        if let Value::Arr(a) = self {
            a.push(v.into());
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        if n.is_finite() {
            Value::Num(n)
        } else {
            Value::Null
        }
    }
}
impl From<f32> for Value {
    fn from(n: f32) -> Self {
        Value::from(n as f64)
    }
}
macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Value { fn from(n: $t) -> Self { Value::Num(n as f64) } }
    )*};
}
from_int!(i8, i16, i32, i64, u8, u16, u32, u64, usize);
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Self {
        Value::Str(s.clone())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        match o {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Arr(v.into_iter().map(Into::into).collect())
    }
}

// ---------------------------------------------------------------- reading

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(*n as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&Vec<(String, Value)>> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }

    // Required / optional accessors with readable errors.
    pub fn req(&self, key: &str) -> JResult<&Value> {
        match self.get(key) {
            Some(v) if !v.is_null() => Ok(v),
            _ => err(format!("missing field '{key}'")),
        }
    }
    pub fn req_f64(&self, key: &str) -> JResult<f64> {
        self.req(key)?
            .as_f64()
            .ok_or_else(|| JsonError(format!("field '{key}' must be a number")))
    }
    pub fn req_i64(&self, key: &str) -> JResult<i64> {
        self.req(key)?
            .as_i64()
            .ok_or_else(|| JsonError(format!("field '{key}' must be an integer")))
    }
    pub fn req_str(&self, key: &str) -> JResult<&str> {
        self.req(key)?
            .as_str()
            .ok_or_else(|| JsonError(format!("field '{key}' must be a string")))
    }
    pub fn req_bool(&self, key: &str) -> JResult<bool> {
        self.req(key)?
            .as_bool()
            .ok_or_else(|| JsonError(format!("field '{key}' must be a boolean")))
    }
    pub fn req_arr(&self, key: &str) -> JResult<&Vec<Value>> {
        self.req(key)?
            .as_arr()
            .ok_or_else(|| JsonError(format!("field '{key}' must be an array")))
    }
    pub fn opt_f64(&self, key: &str) -> JResult<Option<f64>> {
        match self.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Num(n)) => Ok(Some(*n)),
            _ => err(format!("field '{key}' must be a number or null")),
        }
    }
    pub fn opt_i64(&self, key: &str) -> JResult<Option<i64>> {
        match self.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_i64()
                .map(Some)
                .ok_or_else(|| JsonError(format!("field '{key}' must be an integer or null"))),
        }
    }
    pub fn opt_str(&self, key: &str) -> JResult<Option<&str>> {
        match self.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s)),
            _ => err(format!("field '{key}' must be a string or null")),
        }
    }
    pub fn opt_bool(&self, key: &str) -> JResult<Option<bool>> {
        match self.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            _ => err(format!("field '{key}' must be a boolean or null")),
        }
    }
    pub fn str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).and_then(|v| v.as_str()).unwrap_or(default)
    }
    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }
    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }
}

/// Bounded numeric check helper for validating untrusted payloads.
pub fn check_range(name: &str, v: f64, min: f64, max: f64) -> JResult<f64> {
    if !v.is_finite() || v < min || v > max {
        return err(format!("{name} must be between {min} and {max} (got {v})"));
    }
    Ok(v)
}

pub trait ToJson {
    fn to_json(&self) -> Value;
}
pub trait FromJson: Sized {
    fn from_json(v: &Value) -> JResult<Self>;
}

impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> Value {
        Value::Arr(self.iter().map(|x| x.to_json()).collect())
    }
}
impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> Value {
        match self {
            Some(x) => x.to_json(),
            None => Value::Null,
        }
    }
}
pub fn vec_from_json<T: FromJson>(v: &Value) -> JResult<Vec<T>> {
    match v {
        Value::Arr(a) => a.iter().map(T::from_json).collect(),
        Value::Null => Ok(Vec::new()),
        _ => err("expected array"),
    }
}

// ---------------------------------------------------------------- serialize

impl Value {
    pub fn to_string_compact(&self) -> String {
        let mut s = String::new();
        write_value(&mut s, self, None, 0);
        s
    }
    pub fn to_string_pretty(&self) -> String {
        let mut s = String::new();
        write_value(&mut s, self, Some(2), 0);
        s
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_compact())
    }
}

fn write_num(out: &mut String, n: f64) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        out.push_str(&format!("{}", n as i64));
    } else {
        // Rust's shortest round-trip representation.
        let s = format!("{n}");
        out.push_str(&s);
    }
}

pub fn write_str_escaped(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Escape characters that are dangerous inside <script> or HTML contexts.
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn indent(out: &mut String, pretty: Option<usize>, level: usize) {
    if let Some(w) = pretty {
        out.push('\n');
        for _ in 0..(w * level) {
            out.push(' ');
        }
    }
}

fn write_value(out: &mut String, v: &Value, pretty: Option<usize>, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => write_num(out, *n),
        Value::Str(s) => write_str_escaped(out, s),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(out, pretty, level + 1);
                write_value(out, x, pretty, level + 1);
            }
            if !a.is_empty() {
                indent(out, pretty, level);
            }
            out.push(']');
        }
        Value::Obj(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                indent(out, pretty, level + 1);
                write_str_escaped(out, k);
                out.push(':');
                if pretty.is_some() {
                    out.push(' ');
                }
                write_value(out, x, pretty, level + 1);
            }
            if !o.is_empty() {
                indent(out, pretty, level);
            }
            out.push('}');
        }
    }
}

// ---------------------------------------------------------------- parse

pub fn parse(input: &str) -> JResult<Value> {
    if input.len() > MAX_INPUT_BYTES {
        return err("JSON input too large");
    }
    let mut p = Parser { b: input.as_bytes(), i: 0, src: input };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return err(format!("unexpected trailing data at byte {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> JResult<()> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn lit(&mut self, s: &str, v: Value) -> JResult<Value> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Ok(v)
        } else {
            err(format!("invalid literal at byte {}", self.i))
        }
    }
    fn value(&mut self, depth: usize) -> JResult<Value> {
        if depth > MAX_DEPTH {
            return err("JSON nesting too deep");
        }
        match self.peek() {
            None => err("unexpected end of JSON"),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Value::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Arr(a));
                        }
                        _ => return err(format!("expected ',' or ']' at byte {}", self.i)),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut o: Vec<(String, Value)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Value::Obj(o));
                }
                loop {
                    self.ws();
                    if self.peek() != Some(b'"') {
                        return err(format!("expected object key at byte {}", self.i));
                    }
                    let k = self.string()?;
                    self.ws();
                    self.expect(b':')?;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    if let Some(p) = o.iter_mut().find(|(kk, _)| *kk == k) {
                        p.1 = v; // last duplicate wins
                    } else {
                        o.push((k, v));
                    }
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Obj(o));
                        }
                        _ => return err(format!("expected ',' or '}}' at byte {}", self.i)),
                    }
                }
            }
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => err(format!("unexpected character at byte {}", self.i)),
        }
    }
    fn number(&mut self) -> JResult<Value> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        let digits_start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == digits_start {
            return err(format!("invalid number at byte {start}"));
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            let f = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == f {
                return err(format!("invalid number at byte {start}"));
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            let e = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == e {
                return err(format!("invalid number at byte {start}"));
            }
        }
        let n: f64 = self.src[start..self.i]
            .parse()
            .map_err(|_| JsonError(format!("invalid number at byte {start}")))?;
        if !n.is_finite() {
            return err("non-finite number");
        }
        Ok(Value::Num(n))
    }
    fn hex4(&mut self) -> JResult<u32> {
        if self.i + 4 > self.b.len() {
            return err("truncated unicode escape");
        }
        let s = &self.src[self.i..self.i + 4];
        self.i += 4;
        u32::from_str_radix(s, 16).map_err(|_| JsonError("invalid unicode escape".into()))
    }
    fn string(&mut self) -> JResult<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let start = self.i;
            while let Some(c) = self.peek() {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(&self.src[start..self.i]);
            match self.peek() {
                None => return err("unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| JsonError("bad escape".into()))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.b[self.i..].starts_with(b"\\u") {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return err("invalid surrogate pair");
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    return err("lone surrogate");
                                }
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return err("lone surrogate");
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).ok_or_else(|| JsonError("bad code point".into()))?);
                        }
                        _ => return err("invalid escape"),
                    }
                }
                Some(_) => return err("control character in string"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r#"{"a":1,"b":[true,false,null],"c":"x\"y\n","d":-2.5e3,"e":{"f":"\u00e9\ud83d\ude00"}}"#;
        let v = parse(src).unwrap();
        assert_eq!(v.req_f64("a").unwrap(), 1.0);
        assert_eq!(v.get("d").unwrap().as_f64(), Some(-2500.0));
        assert_eq!(v.get("e").unwrap().req_str("f").unwrap(), "é😀");
        let again = parse(&v.to_string_compact()).unwrap();
        assert_eq!(v, again);
        let pretty = parse(&v.to_string_pretty()).unwrap();
        assert_eq!(v, pretty);
    }

    #[test]
    fn rejects_bad_input() {
        for s in ["", "{", "[1,]", "{\"a\" 1}", "01x", "1e999", "\"\\ud800\"", "nul", "[1] 2", "\"a\u{1}\""] {
            assert!(parse(s).is_err(), "should reject {s:?}");
        }
        let deep = "[".repeat(100) + &"]".repeat(100);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn non_finite_serializes_as_null() {
        assert_eq!(Value::from(f64::NAN).to_string_compact(), "null");
        assert_eq!(Value::Num(f64::INFINITY).to_string_compact(), "null");
    }

    #[test]
    fn escapes_html_sensitive() {
        assert_eq!(Value::from("</script>").to_string_compact(), "\"\\u003c/script\\u003e\"");
    }
}
