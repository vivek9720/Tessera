//! A bounds-checked cursor over a byte slice.
//!
//! Every read validates that enough bytes remain before consuming them, so the
//! module loader can decode attacker-controlled input without ever indexing out
//! of range. Multi-byte integers use little-endian fixed encodings; variable
//! length integers use unsigned LEB128, which the compiler emits for indices
//! and jump offsets.

use crate::error::{Error, Result};

/// A forward-only reader over a borrowed byte buffer.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    /// Current read offset from the start of the buffer.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Total length of the underlying buffer.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Number of bytes still available.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// True once every byte has been consumed.
    pub fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn need(&self, n: usize) -> Result<()> {
        if self.remaining() < n {
            return Err(Error::decode(format!(
                "unexpected end of input: needed {} byte(s) at offset {}, {} remaining",
                n,
                self.pos,
                self.remaining()
            )));
        }
        Ok(())
    }

    /// Read a single byte.
    pub fn u8(&mut self) -> Result<u8> {
        self.need(1)?;
        let b = self.buf[self.pos];
        self.pos += 1;
        Ok(b)
    }

    /// Peek at the next byte without consuming it.
    pub fn peek_u8(&self) -> Result<u8> {
        self.need(1)?;
        Ok(self.buf[self.pos])
    }

    /// Read a little-endian `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    /// Read a little-endian `u32`.
    pub fn u32(&mut self) -> Result<u32> {
        self.need(4)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(&self.buf[self.pos..self.pos + 4]);
        self.pos += 4;
        Ok(u32::from_le_bytes(a))
    }

    /// Read a little-endian `u64`.
    pub fn u64(&mut self) -> Result<u64> {
        self.need(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.buf[self.pos..self.pos + 8]);
        self.pos += 8;
        Ok(u64::from_le_bytes(a))
    }

    /// Read a little-endian `i64`.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }

    /// Read an IEEE-754 double stored little-endian.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// Read an unsigned LEB128 integer, capped at 64 bits.
    ///
    /// Rejects encodings longer than ten payload bytes and encodings whose high
    /// bits would not fit in a `u64`, so a hostile stream cannot spin the
    /// decoder or smuggle extra bits.
    pub fn uleb(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8()?;
            let payload = (byte & 0x7f) as u64;
            if shift >= 64 {
                return Err(Error::decode("LEB128 value overflows 64 bits"));
            }
            if shift == 63 && payload > 1 {
                return Err(Error::decode("LEB128 value overflows 64 bits"));
            }
            result |= payload << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(result)
    }

    /// Read a LEB128 integer and narrow it to `usize`, rejecting values that do
    /// not fit (relevant only on 32-bit hosts, but checked unconditionally).
    pub fn uleb_usize(&mut self) -> Result<usize> {
        let v = self.uleb()?;
        usize::try_from(v).map_err(|_| Error::decode("length does not fit in usize"))
    }

    /// Read a length-prefixed byte string: a ULEB length followed by that many
    /// raw bytes.
    pub fn bytes_prefixed(&mut self) -> Result<&'a [u8]> {
        let n = self.uleb_usize()?;
        self.need(n)?;
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Read `n` raw bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        self.need(n)?;
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Read a length-prefixed UTF-8 string. Invalid UTF-8 is a decode error;
    /// Tessera strings in the constant pool are required to be valid UTF-8.
    pub fn utf8_prefixed(&mut self) -> Result<String> {
        let raw = self.bytes_prefixed()?;
        std::str::from_utf8(raw)
            .map(|s| s.to_owned())
            .map_err(|_| Error::decode("string constant is not valid UTF-8"))
    }

    /// Skip `n` bytes without returning them.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.need(n)?;
        self.pos += n;
        Ok(())
    }

    /// Assert that the reader is positioned exactly at the end of the buffer.
    pub fn expect_eof(&self) -> Result<()> {
        if !self.at_end() {
            return Err(Error::decode(format!(
                "{} trailing byte(s) after module body",
                self.remaining()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fixed_integers() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let mut r = Reader::new(&data);
        assert_eq!(r.u16().unwrap(), 0x0201);
        assert_eq!(r.u16().unwrap(), 0x0403);
        assert_eq!(r.remaining(), 4);
    }

    #[test]
    fn uleb_roundtrip_small_and_large() {
        // 300 = 0xAC 0x02 in ULEB128.
        let mut r = Reader::new(&[0xac, 0x02]);
        assert_eq!(r.uleb().unwrap(), 300);
        assert!(r.at_end());
    }

    #[test]
    fn uleb_rejects_overlong() {
        let data = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02];
        let mut r = Reader::new(&data);
        assert!(r.uleb().is_err());
    }

    #[test]
    fn truncated_read_is_error() {
        let mut r = Reader::new(&[0x00]);
        assert!(r.u32().is_err());
    }

    #[test]
    fn prefixed_string_roundtrips() {
        let mut data = vec![5u8];
        data.extend_from_slice(b"hello");
        let mut r = Reader::new(&data);
        assert_eq!(r.utf8_prefixed().unwrap(), "hello");
    }
}
