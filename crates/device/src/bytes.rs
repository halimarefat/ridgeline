//! Little-endian byte reader/writer used by the GATT parsers.
//! Bluetooth GATT multi-octet fields are little-endian.

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    Truncated { needed: usize, at: usize, len: usize },
    Empty,
    Invalid(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Truncated { needed, at, len } => {
                write!(f, "packet truncated: need {needed} byte(s) at offset {at}, length {len}")
            }
            ParseError::Empty => write!(f, "empty packet"),
            ParseError::Invalid(s) => write!(f, "invalid packet: {s}"),
        }
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        if self.pos + n > self.b.len() {
            return Err(ParseError::Truncated { needed: n, at: self.pos, len: self.b.len() });
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, ParseError> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn i16(&mut self) -> Result<i16, ParseError> {
        let s = self.take(2)?;
        Ok(i16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u24(&mut self) -> Result<u32, ParseError> {
        let s = self.take(3)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], 0]))
    }
    pub fn u32(&mut self) -> Result<u32, ParseError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn skip(&mut self, n: usize) -> Result<(), ParseError> {
        self.take(n).map(|_| ())
    }
    pub fn remaining(&self) -> usize {
        self.b.len() - self.pos
    }
}

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u24(&mut self, v: u32) -> &mut Self {
        let b = v.to_le_bytes();
        self.buf.extend_from_slice(&b[0..3]);
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn done(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() % 2 != 0 {
        return None;
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok())
        .collect()
}
