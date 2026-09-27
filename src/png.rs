//! Minimal dependency-free PNG writer (8-bit RGB/RGBA, no interlace, stored deflate).
//!
//! Stored (uncompressed) deflate keeps the encoder small and obviously correct; the
//! files are larger than `libpng` output but every viewer reads them.

use std::io;
use std::path::Path;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const MAX_BLOCK: usize = 65_535;

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// zlib stream wrapper around stored deflate blocks.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01];
    let mut pos = 0;
    while pos < raw.len() {
        let n = (raw.len() - pos).min(MAX_BLOCK);
        let last = pos + n == raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(n as u16).to_le_bytes());
        z.extend_from_slice(&(!(n as u16)).to_le_bytes());
        z.extend_from_slice(&raw[pos..pos + n]);
        pos += n;
    }
    if raw.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    z.extend_from_slice(&adler32(raw).to_be_bytes());
    z
}

fn encode(width: u32, height: u32, channels: u8, pixels: &[u8]) -> Vec<u8> {
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    let stride = w * channels as usize;
    assert_eq!(pixels.len(), stride * h, "pixel buffer size does not match {w}x{h}x{channels}");

    let mut raw = Vec::with_capacity(h * (1 + stride));
    for y in 0..h {
        raw.push(0); // filter type: none
        raw.extend_from_slice(&pixels[y * stride..(y + 1) * stride]);
    }

    let mut out = Vec::with_capacity(raw.len() + 128);
    out.extend_from_slice(&SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.max(1).to_be_bytes());
    ihdr.extend_from_slice(&height.max(1).to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    encode(width, height, 3, rgb)
}

/// Encode RGBA pixels, optionally flipping rows (GL framebuffers are bottom-up).
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8], flip_y: bool) -> Vec<u8> {
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    if !flip_y {
        return encode(width, height, 4, rgba);
    }
    let mut flipped = Vec::with_capacity(w * h * 4);
    for y in (0..h).rev() {
        flipped.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }
    encode(width, height, 4, &flipped)
}

pub fn write_rgb(path: &str, width: u32, height: u32, rgb: &[u8]) -> io::Result<()> {
    if let Some(dir) = Path::new(path).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(path, encode_rgb(width, height, rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_reference_vectors() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn header_and_chunks_are_well_formed() {
        let rgb = vec![10u8; 3 * 4 * 2];
        let png = encode_rgb(4, 2, &rgb);
        assert_eq!(&png[..8], &SIGNATURE);
        assert_eq!(&png[8..12], &[0, 0, 0, 13]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), 4);
        assert_eq!(u32::from_be_bytes([png[20], png[21], png[22], png[23]]), 2);
        assert_eq!(&png[24..29], &[8, 2, 0, 0, 0]);
        assert_eq!(&png[png.len() - 12..png.len() - 8], &[0, 0, 0, 0]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        assert_eq!(&png[png.len() - 4..], &[0xae, 0x42, 0x60, 0x82]); // CRC of "IEND"
    }

    #[test]
    fn rgba_flip_reverses_rows() {
        let mut rgba = vec![0u8; 4 * 2 * 2];
        rgba[..4].copy_from_slice(&[1, 2, 3, 4]); // first row
        let plain = encode_rgba(2, 2, &rgba, false);
        let flipped = encode_rgba(2, 2, &rgba, true);
        assert_ne!(plain, flipped);
        // The tagged row must be the first row in `plain` and the last row in `flipped`.
        let first_tag = |buf: &[u8]| {
            buf.windows(4).position(|w| w == [1, 2, 3, 4]).expect("tagged row present")
        };
        let (p0, p1) = (first_tag(&plain), first_tag(&flipped));
        assert!(p0 > 0 && p1 > 0);
        assert!(p1 > p0, "flipped image must place the tagged row later ({p0} vs {p1})");
    }
}
