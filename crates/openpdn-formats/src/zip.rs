//! Minimal ZIP reading/writing (stored and deflated entries, no ZIP64),
//! enough for `.openpdn` documents.

use crate::checksum::crc32;
use crate::{err, Result};

pub struct ZipEntry<'a> {
    pub name: String,
    method: u16,
    compressed: &'a [u8],
    crc: u32,
    size: u32,
}

impl ZipEntry<'_> {
    /// Decompressed bytes, CRC-checked.
    pub fn data(&self) -> Result<Vec<u8>> {
        let out = match self.method {
            0 => self.compressed.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(self.compressed, self.size as usize)
                .map_err(|_| crate::FormatError(format!("corrupt zip entry {}", self.name)))?,
            m => return err(format!("unsupported zip compression method {m}")),
        };
        if out.len() != self.size as usize || crc32(&out) != self.crc {
            return err(format!("zip entry {} failed its integrity check", self.name));
        }
        Ok(out)
    }
}

fn u16le(b: &[u8], o: usize) -> Result<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| crate::FormatError("truncated zip".into()))
}

fn u32le(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| crate::FormatError("truncated zip".into()))
}

/// Lists entries via the central directory (so data descriptors are fine).
pub fn read(data: &[u8]) -> Result<Vec<ZipEntry<'_>>> {
    // End of central directory: last occurrence of PK\5\6 within the final 64 KB + 22.
    let start = data.len().saturating_sub(65535 + 22);
    let eocd = (start..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..i + 4] == *b"PK\x05\x06")
        .ok_or_else(|| crate::FormatError("not a zip archive".into()))?;
    let count = u16le(data, eocd + 10)? as usize;
    let mut p = u32le(data, eocd + 16)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if u32le(data, p)? != 0x02014b50 {
            return err("corrupt zip central directory");
        }
        let method = u16le(data, p + 10)?;
        let crc = u32le(data, p + 16)?;
        let csize = u32le(data, p + 20)? as usize;
        let size = u32le(data, p + 24)?;
        let nlen = u16le(data, p + 28)? as usize;
        let xlen = u16le(data, p + 30)? as usize;
        let clen = u16le(data, p + 32)? as usize;
        let local = u32le(data, p + 42)? as usize;
        let name = String::from_utf8_lossy(data.get(p + 46..p + 46 + nlen).ok_or_else(|| crate::FormatError("truncated zip".into()))?)
            .into_owned();
        p += 46 + nlen + xlen + clen;
        if u32le(data, local)? != 0x04034b50 {
            return err("corrupt zip local header");
        }
        let body = local + 30 + u16le(data, local + 26)? as usize + u16le(data, local + 28)? as usize;
        let compressed = data.get(body..body + csize).ok_or_else(|| crate::FormatError("truncated zip entry".into()))?;
        entries.push(ZipEntry { name, method, compressed, crc, size });
    }
    Ok(entries)
}

/// Builds a zip archive in memory.
#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an entry; `deflate` compresses it (skip for already-compressed data like PNG).
    pub fn add(&mut self, name: &str, data: &[u8], deflate: bool) -> Result<()> {
        if data.len() > u32::MAX as usize || self.out.len() > u32::MAX as usize {
            return err("document too large for a zip without ZIP64");
        }
        let crc = crc32(data);
        let packed;
        let (method, body): (u16, &[u8]) = if deflate {
            packed = miniz_oxide::deflate::compress_to_vec(data, 6);
            (8, &packed)
        } else {
            (0, data)
        };
        let offset = self.out.len() as u32;
        let n = name.as_bytes();
        let mut local = Vec::with_capacity(30 + n.len());
        local.extend_from_slice(&0x04034b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes()); // version needed
        local.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        local.extend_from_slice(&method.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes()); // time
        local.extend_from_slice(&0x0021u16.to_le_bytes()); // date 1980-01-01
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(body.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(n.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(n);
        self.out.extend_from_slice(&local);
        self.out.extend_from_slice(body);

        let c = &mut self.central;
        c.extend_from_slice(&0x02014b50u32.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes()); // made by
        c.extend_from_slice(&local[4..30]); // shared fields
        c.extend_from_slice(&0u16.to_le_bytes()); // comment len
        c.extend_from_slice(&0u16.to_le_bytes()); // disk
        c.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        c.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(n);
        self.count += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.out.len() as u32;
        let cd_len = self.central.len() as u32;
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x06054b50u32.to_le_bytes());
        self.out.extend_from_slice(&[0, 0, 0, 0]);
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&cd_len.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = ZipWriter::new();
        w.add("a.txt", b"hello hello hello hello", true).unwrap();
        w.add("dir/b.bin", &[1, 2, 3], false).unwrap();
        let z = w.finish();
        let e = read(&z).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "a.txt");
        assert_eq!(e[0].data().unwrap(), b"hello hello hello hello");
        assert_eq!(e[1].data().unwrap(), vec![1, 2, 3]);
    }
}
