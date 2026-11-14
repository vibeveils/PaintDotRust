//! CRC-32 (slicing-by-8) and Adler-32 with combine.

const fn crc_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[0][n] = c;
        n += 1;
    }
    let mut n = 0;
    while n < 256 {
        let mut k = 1;
        while k < 8 {
            t[k][n] = (t[k - 1][n] >> 8) ^ t[0][(t[k - 1][n] & 0xFF) as usize];
            k += 1;
        }
        n += 1;
    }
    t
}

static CRC: [[u32; 256]; 8] = crc_tables();

/// Continues a CRC-32 (pass the running value, pre-inverted state as in zlib's `crc32(c, …)`).
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    let mut chunks = data.chunks_exact(8);
    for ch in &mut chunks {
        let lo = u32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]) ^ c;
        let hi = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        c = CRC[7][(lo & 0xFF) as usize]
            ^ CRC[6][((lo >> 8) & 0xFF) as usize]
            ^ CRC[5][((lo >> 16) & 0xFF) as usize]
            ^ CRC[4][(lo >> 24) as usize]
            ^ CRC[3][(hi & 0xFF) as usize]
            ^ CRC[2][((hi >> 8) & 0xFF) as usize]
            ^ CRC[1][((hi >> 16) & 0xFF) as usize]
            ^ CRC[0][(hi >> 24) as usize];
    }
    for &b in chunks.remainder() {
        c = CRC[0][((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

const ADLER_MOD: u32 = 65521;

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for block in data.chunks(5552) {
        for &x in block {
            a += x as u32;
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

/// Adler-32 of A‖B from adler(A), adler(B) and len(B) (zlib's `adler32_combine`).
pub fn adler32_combine(adler1: u32, adler2: u32, len2: u64) -> u32 {
    let rem = (len2 % ADLER_MOD as u64) as u32;
    let mut sum1 = adler1 & 0xffff;
    let mut sum2 = (rem * sum1) % ADLER_MOD;
    sum1 += (adler2 & 0xffff) + ADLER_MOD - 1;
    sum2 += ((adler1 >> 16) & 0xffff) + ((adler2 >> 16) & 0xffff) + ADLER_MOD - rem;
    if sum1 >= ADLER_MOD {
        sum1 -= ADLER_MOD;
    }
    if sum1 >= ADLER_MOD {
        sum1 -= ADLER_MOD;
    }
    if sum2 >= ADLER_MOD << 1 {
        sum2 -= ADLER_MOD << 1;
    }
    if sum2 >= ADLER_MOD {
        sum2 -= ADLER_MOD;
    }
    (sum2 << 16) | sum1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E60398);
        let (a, b) = (b"hello, ".as_slice(), b"world and more bytes".as_slice());
        let whole: Vec<u8> = [a, b].concat();
        assert_eq!(adler32_combine(adler32(a), adler32(b), b.len() as u64), adler32(&whole));
        assert_eq!(crc32_update(crc32(a), b), crc32(&whole));
    }
}
