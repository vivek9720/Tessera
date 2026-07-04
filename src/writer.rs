//! A growable byte sink used by the module serializer.
//!
//! The encoding mirrors [`crate::reader::Reader`]: little-endian fixed integers
//! and unsigned LEB128 for variable-width values. The serializer is used to
//! produce module files from compiled programs and to build fixtures for the
//! test suite, so it is deliberately simple and allocation-friendly.

/// A byte buffer with typed append helpers.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { buf: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Writer {
        Writer { buf: Vec::with_capacity(cap) }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Consume the writer and return the accumulated bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Borrow the accumulated bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.u64(v as u64)
    }

    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.u64(v.to_bits())
    }

    /// Append an unsigned LEB128 integer.
    pub fn uleb(&mut self, mut v: u64) -> &mut Self {
        loop {
            let mut byte = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
            }
            self.buf.push(byte);
            if v == 0 {
                break;
            }
        }
        self
    }

    pub fn uleb_usize(&mut self, v: usize) -> &mut Self {
        self.uleb(v as u64)
    }

    /// Append a length-prefixed raw byte string.
    pub fn bytes_prefixed(&mut self, bytes: &[u8]) -> &mut Self {
        self.uleb_usize(bytes.len());
        self.buf.extend_from_slice(bytes);
        self
    }

    /// Append a length-prefixed UTF-8 string.
    pub fn utf8_prefixed(&mut self, s: &str) -> &mut Self {
        self.bytes_prefixed(s.as_bytes())
    }

    /// Append raw bytes with no length prefix.
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    /// Reserve a 4-byte little-endian slot and return its offset so it can be
    /// backpatched once the true value is known (used for section sizes).
    pub fn reserve_u32(&mut self) -> usize {
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        at
    }

    /// Overwrite a previously reserved 4-byte slot.
    pub fn patch_u32(&mut self, at: usize, v: u32) {
        let bytes = v.to_le_bytes();
        self.buf[at..at + 4].copy_from_slice(&bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::Reader;

    #[test]
    fn leb_roundtrips_through_reader() {
        let mut w = Writer::new();
        for v in [0u64, 1, 127, 128, 300, 16384, u32::MAX as u64, u64::MAX] {
            w.uleb(v);
        }
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        for v in [0u64, 1, 127, 128, 300, 16384, u32::MAX as u64, u64::MAX] {
            assert_eq!(r.uleb().unwrap(), v);
        }
        assert!(r.at_end());
    }

    #[test]
    fn backpatch_writes_size() {
        let mut w = Writer::new();
        let slot = w.reserve_u32();
        w.raw(&[1, 2, 3]);
        w.patch_u32(slot, 3);
        let bytes = w.into_bytes();
        assert_eq!(&bytes[0..4], &3u32.to_le_bytes());
    }

    #[test]
    fn string_roundtrips() {
        let mut w = Writer::new();
        w.utf8_prefixed("tessera");
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.utf8_prefixed().unwrap(), "tessera");
    }
}
