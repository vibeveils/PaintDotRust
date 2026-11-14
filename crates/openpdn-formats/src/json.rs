//! A small JSON value type with a parser and writer, for document manifests.

use crate::{err, Result};
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            // .NET's deserializer matches property names case-insensitively only when asked;
            // the manifests we read are written with exact names, but tolerate case anyway.
            Json::Obj(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        if let Json::Num(n) = self { Some(*n) } else { None }
    }
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64)
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self { Some(*b) } else { None }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        if let Json::Arr(a) = self { Some(a) } else { None }
    }

    pub fn parse(text: &str) -> Result<Json> {
        let mut p = Parser { s: text.as_bytes(), i: 0, depth: 0 };
        // Allow a UTF-8 BOM.
        if p.s.starts_with(&[0xEF, 0xBB, 0xBF]) {
            p.i = 3;
        }
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return err("trailing characters after JSON");
        }
        Ok(v)
    }

    /// Indented output (two spaces), like `WriteIndented = true`.
    pub fn to_string_pretty(&self) -> String {
        let mut s = String::new();
        self.write(&mut s, 0);
        s
    }

    fn write(&self, out: &mut String, indent: usize) {
        let pad = |out: &mut String, n: usize| out.extend(std::iter::repeat_n(' ', n * 2));
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            Json::Str(s) => write_str(out, s),
            Json::Arr(a) if a.is_empty() => out.push_str("[]"),
            Json::Arr(a) => {
                out.push_str("[\n");
                for (i, v) in a.iter().enumerate() {
                    pad(out, indent + 1);
                    v.write(out, indent + 1);
                    out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push(']');
            }
            Json::Obj(m) if m.is_empty() => out.push_str("{}"),
            Json::Obj(m) => {
                out.push_str("{\n");
                for (i, (k, v)) in m.iter().enumerate() {
                    pad(out, indent + 1);
                    write_str(out, k);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                    out.push_str(if i + 1 < m.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push('}');
            }
        }
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Json> {
        self.ws();
        let Some(&c) = self.s.get(self.i) else { return err("unexpected end of JSON") };
        match c {
            b'n' if self.eat("null") => Ok(Json::Null),
            b't' if self.eat("true") => Ok(Json::Bool(true)),
            b'f' if self.eat("false") => Ok(Json::Bool(false)),
            b'"' => Ok(Json::Str(self.string()?)),
            b'[' | b'{' => {
                self.depth += 1;
                if self.depth > 128 {
                    return err("JSON nested too deeply");
                }
                let v = if c == b'[' { self.array() } else { self.object() };
                self.depth -= 1;
                v
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => err("invalid JSON"),
        }
    }

    fn array(&mut self) -> Result<Json> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.eat("]") {
            return Ok(Json::Arr(a));
        }
        loop {
            a.push(self.value()?);
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("]") {
                return Ok(Json::Arr(a));
            }
            return err("expected , or ] in JSON array");
        }
    }

    fn object(&mut self) -> Result<Json> {
        self.i += 1;
        let mut m = Vec::new();
        self.ws();
        if self.eat("}") {
            return Ok(Json::Obj(m));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return err("expected key in JSON object");
            }
            let k = self.string()?;
            self.ws();
            if !self.eat(":") {
                return err("expected : in JSON object");
            }
            m.push((k, self.value()?));
            self.ws();
            if self.eat(",") {
                continue;
            }
            if self.eat("}") {
                return Ok(Json::Obj(m));
            }
            return err("expected , or } in JSON object");
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| crate::FormatError("bad JSON escape".into()))?;
        let v = u32::from_str_radix(std::str::from_utf8(h).unwrap_or("x"), 16)
            .map_err(|_| crate::FormatError("bad JSON escape".into()))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return err("unterminated JSON string") };
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| crate::FormatError("invalid UTF-8 in JSON".into())),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return err("bad JSON escape") };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        _ => return err("bad JSON escape"),
                    };
                    let mut b = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }

    fn number(&mut self) -> Result<Json> {
        let st = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[st..self.i])
            .ok()
            .and_then(|t| t.parse().ok())
            .map(Json::Num)
            .ok_or_else(|| crate::FormatError("bad JSON number".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write() {
        let src = r#"{"a": [1, 2.5, -3e2], "b": "x\"yé😀", "c": true, "d": null, "e": {}}"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[2].as_f64(), Some(-300.0));
        assert_eq!(v.get("b").unwrap().as_str(), Some("x\"yé😀"));
        let again = Json::parse(&v.to_string_pretty()).unwrap();
        assert_eq!(again, v);
        assert!(Json::parse("[1,]").is_err());
    }
}
