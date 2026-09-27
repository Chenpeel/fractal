//! Zero-dependency animated GIF89a writer.
//!
//! Streaming by design: `create` writes the header, palette and loop extension, each
//! `add_frame` quantises one RGB8 frame and appends it as an LZW-compressed image, and
//! `finish` writes the trailer. Frames are never buffered in memory.

use std::fs::File;
use std::io::{self, BufWriter, Write};

const CUBE_LEVELS: usize = 6;
const DARK: usize = 8;
const GRAYS: usize = 32;
const TABLE_SIZE: usize = 256;
const CLEAR_CODE: u16 = 256;
const EOI_CODE: u16 = 257;
const FIRST_FREE: u16 = 258;
const MAX_CODE: u16 = 4095;
const EMPTY: u16 = u16::MAX;
const LUT_BITS: usize = 5; // 32 levels per channel for quantisation lookup
const LUT_SIZE: usize = 1 << (3 * LUT_BITS);

/// Fixed palette: dark background shades, a grey ramp, then a 6x6x6 RGB cube.
fn palette() -> [[u8; 3]; TABLE_SIZE] {
    const DARK_COLORS: [[u8; 3]; DARK] = [
        [0, 0, 0],
        [1, 1, 2],
        [2, 3, 6],
        [3, 4, 8],
        [4, 5, 11], // exact renderer background
        [6, 8, 17],
        [8, 10, 22],
        [10, 13, 28],
    ];
    let mut p = [[0u8; 3]; TABLE_SIZE];
    p[..DARK].copy_from_slice(&DARK_COLORS);
    for i in 0..GRAYS {
        let v = (i as u32 * 255 / (GRAYS as u32 - 1)) as u8;
        p[DARK + i] = [v, v, v];
    }
    let mut idx = DARK + GRAYS;
    let step = |v: usize| (v as u32 * 255 / (CUBE_LEVELS as u32 - 1)) as u8;
    for r in 0..CUBE_LEVELS {
        for g in 0..CUBE_LEVELS {
            for b in 0..CUBE_LEVELS {
                p[idx] = [step(r), step(g), step(b)];
                idx += 1;
            }
        }
    }
    debug_assert_eq!(idx, TABLE_SIZE);
    p
}

fn bin_centre(slot: usize) -> (i32, i32, i32) {
    let centre = |v: usize| (v * 255 / ((1 << LUT_BITS) - 1)) as i32;
    (
        centre((slot >> (2 * LUT_BITS)) & ((1 << LUT_BITS) - 1)),
        centre((slot >> LUT_BITS) & ((1 << LUT_BITS) - 1)),
        centre(slot & ((1 << LUT_BITS) - 1)),
    )
}

/// 32x32x32 nearest-palette lookup, built once per file (~8M distance tests).
fn build_lut(palette: &[[u8; 3]; TABLE_SIZE]) -> Vec<u8> {
    let mut lut = vec![0u8; LUT_SIZE];
    for (slot, out) in lut.iter_mut().enumerate() {
        let (cr, cg, cb) = bin_centre(slot);
        let mut best = 0usize;
        let mut best_d = i32::MAX;
        for (i, c) in palette.iter().enumerate() {
            let dr = cr - c[0] as i32;
            let dg = cg - c[1] as i32;
            let db = cb - c[2] as i32;
            let d = dr * dr + dg * dg + db * db;
            if d < best_d {
                best_d = d;
                best = i;
                if d == 0 {
                    break;
                }
            }
        }
        *out = best as u8;
    }
    // A palette colour that sits near its own bin centre owns that bin, so smooth
    // gradients (the dark background ramp, the grey ramp) stay exact.
    let mut owner: Vec<(i32, u8)> = vec![(i32::MAX, 0); LUT_SIZE];
    for (i, c) in palette.iter().enumerate() {
        let slot = lut_slot(c[0], c[1], c[2]);
        let (cr, cg, cb) = bin_centre(slot);
        let d = (cr - c[0] as i32).pow(2) + (cg - c[1] as i32).pow(2) + (cb - c[2] as i32).pow(2);
        if d <= 3 * 12 * 12 && d < owner[slot].0 {
            owner[slot] = (d, i as u8);
        }
    }
    for (slot, (d, i)) in owner.iter().enumerate() {
        if *d != i32::MAX {
            lut[slot] = *i;
        }
    }
    lut
}

#[inline]
fn lut_slot(r: u8, g: u8, b: u8) -> usize {
    let shift = 8 - LUT_BITS;
    ((r as usize >> shift) << (2 * LUT_BITS))
        | ((g as usize >> shift) << LUT_BITS)
        | (b as usize >> shift)
}

/// Reusable LZW encoder state (flat prefix table, LSB-first bit packing).
struct Lzw {
    table: Vec<u16>,
    out: Vec<u8>,
    bits: u32,
    bit_count: u32,
    code_size: u32,
    next_code: u16,
}

impl Lzw {
    fn new() -> Self {
        Lzw {
            table: vec![EMPTY; (MAX_CODE as usize + 1) * 256],
            out: Vec::new(),
            bits: 0,
            bit_count: 0,
            code_size: 9,
            next_code: FIRST_FREE,
        }
    }

    fn emit(&mut self, code: u16) {
        self.bits |= (code as u32) << self.bit_count;
        self.bit_count += self.code_size;
        while self.bit_count >= 8 {
            self.out.push((self.bits & 0xff) as u8);
            self.bits >>= 8;
            self.bit_count -= 8;
        }
    }

    fn reset(&mut self) {
        self.table.fill(EMPTY);
        self.next_code = FIRST_FREE;
        self.code_size = 9;
    }

    /// Encode one frame of palette indices.
    fn encode(&mut self, indices: &[u8]) -> &[u8] {
        self.out.clear();
        self.bits = 0;
        self.bit_count = 0;
        self.reset();
        self.emit(CLEAR_CODE);
        if indices.is_empty() {
            self.emit(EOI_CODE);
        } else {
            let mut prefix = indices[0] as u16;
            for &k in &indices[1..] {
                let key = (prefix as usize) << 8 | k as usize;
                let entry = self.table[key];
                if entry != EMPTY {
                    prefix = entry;
                    continue;
                }
                self.emit(prefix);
                self.table[key] = self.next_code;
                self.next_code += 1;
                if self.next_code > MAX_CODE {
                    self.emit(CLEAR_CODE);
                    self.reset();
                } else if self.next_code > (1 << self.code_size) && self.code_size < 12 {
                    // Bump one emission later than the table suggests: the decoder's
                    // dictionary lags the encoder by exactly one entry.
                    self.code_size += 1;
                }
                prefix = k as u16;
            }
            self.emit(prefix);
            self.emit(EOI_CODE);
        }
        if self.bit_count > 0 {
            self.out.push((self.bits & 0xff) as u8);
        }
        &self.out
    }
}

fn write_sub_blocks(out: &mut impl Write, data: &[u8]) -> io::Result<()> {
    for chunk in data.chunks(255) {
        out.write_all(&[chunk.len() as u8])?;
        out.write_all(chunk)?;
    }
    out.write_all(&[0])
}

pub struct GifWriter {
    out: BufWriter<File>,
    w: u16,
    h: u16,
    delay_cs: u16,
    frames: u32,
    lut: Vec<u8>,
    indices: Vec<u8>,
    lzw: Lzw,
}

impl GifWriter {
    pub fn create(path: &str, w: u32, h: u32, delay_cs: u16) -> io::Result<Self> {
        let w = w.clamp(1, u16::MAX as u32) as u16;
        let h = h.clamp(1, u16::MAX as u32) as u16;
        if let Some(dir) = std::path::Path::new(path).parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let pal = palette();
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(b"GIF89a")?;
        // Logical screen descriptor: global colour table, 8-bit colour resolution, 256 entries.
        out.write_all(&w.to_le_bytes())?;
        out.write_all(&h.to_le_bytes())?;
        out.write_all(&[0xf7, 4, 0])?; // bg index 4 = the renderer background
        for c in pal.iter() {
            out.write_all(c)?;
        }
        // NETSCAPE2.0 application extension -> loop forever.
        out.write_all(&[0x21, 0xff, 0x0b])?;
        out.write_all(b"NETSCAPE2.0")?;
        out.write_all(&[0x03, 0x01, 0x00, 0x00, 0x00])?;
        Ok(GifWriter {
            out,
            w,
            h,
            delay_cs: delay_cs.max(1),
            frames: 0,
            lut: build_lut(&pal),
            indices: vec![0u8; w as usize * h as usize],
            lzw: Lzw::new(),
        })
    }

    pub fn add_frame(&mut self, rgb: &[u8]) -> io::Result<()> {
        let expected = self.w as usize * self.h as usize * 3;
        if rgb.len() != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("frame is {} bytes, expected {expected}", rgb.len()),
            ));
        }
        for (i, px) in rgb.chunks_exact(3).enumerate() {
            self.indices[i] = self.lut[lut_slot(px[0], px[1], px[2])];
        }
        let w = self.w;
        let h = self.h;
        let delay = self.delay_cs;
        let data = self.lzw.encode(&self.indices);
        // Graphic control extension: disposal "do not dispose", no transparency.
        self.out.write_all(&[0x21, 0xf9, 0x04, 0x04])?;
        self.out.write_all(&delay.to_le_bytes())?;
        self.out.write_all(&[0x00, 0x00])?;
        // Image descriptor: full frame, no local colour table, not interlaced.
        self.out.write_all(&[0x2c, 0x00, 0x00, 0x00, 0x00])?;
        self.out.write_all(&w.to_le_bytes())?;
        self.out.write_all(&h.to_le_bytes())?;
        self.out.write_all(&[0x00])?;
        self.out.write_all(&[8])?; // LZW minimum code size
        write_sub_blocks(&mut self.out, data)?;
        self.frames += 1;
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<()> {
        if self.frames == 0 {
            let blank = vec![0u8; self.w as usize * self.h as usize * 3];
            self.add_frame(&blank)?;
        }
        self.out.write_all(&[0x3b])?;
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent LZW decoder written from the GIF89a spec, used to validate the encoder.
    fn lzw_decode(data: &[u8], min_code_size: u32) -> Vec<u8> {
        let clear = 1u16 << min_code_size;
        let eoi = clear + 1;
        let mut dict: Vec<Vec<u8>> = (0..=255u16).map(|i| vec![i as u8]).collect();
        dict.push(Vec::new()); // clear
        dict.push(Vec::new()); // eoi
        let mut code_size = min_code_size + 1;
        let mut out = Vec::new();
        let mut prev: Option<Vec<u8>> = None;
        let mut bits = 0u32;
        let mut bit_count = 0u32;
        let mut pos = 0usize;
        loop {
            while bit_count < code_size && pos < data.len() {
                bits |= (data[pos] as u32) << bit_count;
                bit_count += 8;
                pos += 1;
            }
            if bit_count < code_size {
                break;
            }
            let code = (bits & ((1 << code_size) - 1)) as u16;
            bits >>= code_size;
            bit_count -= code_size;
            if code == clear {
                dict.truncate(clear as usize + 2);
                code_size = min_code_size + 1;
                prev = None;
                continue;
            }
            if code == eoi {
                break;
            }
            let entry = if (code as usize) < dict.len() {
                dict[code as usize].clone()
            } else {
                // KwKwK case: the code refers to the entry being defined right now.
                let base = prev.clone().expect("code beyond dictionary without prefix");
                let mut e = base.clone();
                e.push(base[0]);
                e
            };
            out.extend_from_slice(&entry);
            if let Some(p) = prev {
                let mut new = p;
                new.push(entry[0]);
                dict.push(new);
                if dict.len() >= (1 << code_size) && code_size < 12 {
                    code_size += 1;
                }
            }
            prev = Some(entry);
        }
        out
    }

    fn encode_indices(indices: &[u8]) -> Vec<u8> {
        Lzw::new().encode(indices).to_vec()
    }

    #[test]
    fn palette_has_the_expected_anchors() {
        let p = palette();
        assert_eq!(p[4], [4, 5, 11]);
        assert_eq!(p[DARK], [0, 0, 0]);
        assert_eq!(p[DARK + GRAYS - 1], [255, 255, 255]);
        assert_eq!(p[DARK + GRAYS], [0, 0, 0]);
        assert_eq!(p[255], [255, 255, 255]);
        let mut seen: Vec<[u8; 3]> = p.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() >= 250, "palette too redundant: {} unique", seen.len());
    }

    #[test]
    fn lut_stays_close_to_palette_colors() {
        let p = palette();
        let lut = build_lut(&p);
        for (i, c) in p.iter().enumerate() {
            let got = p[lut[lut_slot(c[0], c[1], c[2])] as usize];
            for k in 0..3 {
                let d = (got[k] as i32 - c[k] as i32).abs();
                assert!(d <= 12, "palette #{i} {c:?} quantised to {got:?} (delta {d})");
            }
        }
        // The renderer background must stay essentially exact and very dark.
        let bg = lut[lut_slot(4, 5, 11)] as usize;
        for k in 0..3 {
            assert!(
                (p[bg][k] as i32 - [4, 5, 11][k]).abs() <= 8,
                "background drifted to {:?}",
                p[bg]
            );
        }
    }

    #[test]
    fn lzw_round_trips_short_and_long_inputs() {
        for len in [1usize, 2, 255, 4096, 60_000] {
            let mut idx: Vec<u8> = Vec::with_capacity(len);
            let mut state: u32 = 12345;
            for _ in 0..len {
                // Deterministic pseudo-random indices: forces table growth and resets.
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                idx.push(((state >> 16) & 0xff) as u8);
            }
            let coded = encode_indices(&idx);
            assert_eq!(lzw_decode(&coded, 8), idx, "round trip failed at len {len}");
        }
    }

    #[test]
    fn lzw_round_trips_compressible_input() {
        let idx: Vec<u8> = (0..50_000).map(|i| ((i / 97) % 8) as u8).collect();
        let coded = encode_indices(&idx);
        assert!(coded.len() < idx.len(), "expected compression, {} vs {}", coded.len(), idx.len());
        assert_eq!(lzw_decode(&coded, 8), idx);
    }

    /// Structural walk of the GIF container: returns the number of image descriptors.
    /// Panics on malformed block structure, so it doubles as a format check.
    fn walk_container(bytes: &[u8]) -> usize {
        assert_eq!(&bytes[..6], b"GIF89a");
        assert_eq!(bytes[10] & 0x80, 0x80, "global colour table flag must be set");
        let gct = 3 * (1 << ((bytes[10] & 0x07) + 1));
        let mut i = 13 + gct;
        let mut frames = 0;
        loop {
            match bytes[i] {
                0x3b => {
                    i += 1;
                    assert_eq!(i, bytes.len(), "trailing bytes after the trailer");
                    break;
                }
                0x21 => {
                    i += 2;
                    let n = bytes[i] as usize;
                    i += 1 + n;
                    while bytes[i] != 0 {
                        let n = bytes[i] as usize;
                        i += 1 + n;
                    }
                    i += 1;
                }
                0x2c => {
                    frames += 1;
                    i += 10; // image descriptor
                    i += 1; // LZW minimum code size
                    while bytes[i] != 0 {
                        let n = bytes[i] as usize;
                        i += 1 + n;
                    }
                    i += 1;
                }
                other => panic!("unexpected block introducer {other:#x} at byte {i}"),
            }
        }
        frames
    }

    #[test]
    fn writer_emits_a_valid_container() {
        let path = std::env::temp_dir().join("ffbd_gif_unit.gif");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);
        let (w, h) = (13u32, 7u32);
        let mut writer = GifWriter::create(&path, w, h, 5).expect("create");
        let mut frame = vec![0u8; (w * h * 3) as usize];
        frame[..3].copy_from_slice(&[4, 5, 11]);
        frame[3..6].copy_from_slice(&[255, 0, 0]);
        writer.add_frame(&frame).expect("frame 1");
        writer.add_frame(&frame).expect("frame 2");
        writer.finish().expect("finish");

        let bytes = std::fs::read(&path).expect("read gif");
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]) as u32, w);
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]) as u32, h);
        assert_eq!(bytes[10], 0xf7);
        assert_eq!(bytes[11], 4, "background index");
        assert_eq!(&bytes[13 + 12..13 + 15], &[4, 5, 11][..], "palette index 4 must be the background");
        assert!(bytes.windows(11).any(|w| w == b"NETSCAPE2.0"), "missing loop extension");
        assert_eq!(walk_container(&bytes), 2);
        // Frame header layout: GCE then a full-frame image descriptor.
        let gct = 3 * (1 << ((bytes[10] & 0x07) + 1));
        let after_ext = 13 + gct + 19;
        assert_eq!(&bytes[after_ext..after_ext + 4], &[0x21, 0xf9, 0x04, 0x04]);
        assert_eq!(u16::from_le_bytes([bytes[after_ext + 4], bytes[after_ext + 5]]), 5);
        let image = after_ext + 8;
        assert_eq!(bytes[image], 0x2c);
        assert_eq!(bytes[image + 9], 0x00, "no local table, not interlaced");
        assert_eq!(u16::from_le_bytes([bytes[image + 5], bytes[image + 6]]) as u32, w);
        assert_eq!(u16::from_le_bytes([bytes[image + 7], bytes[image + 8]]) as u32, h);
        assert_eq!(bytes[image + 10], 8, "LZW minimum code size");
    }

    #[test]
    fn rejected_frame_size_is_an_error() {
        let path = std::env::temp_dir().join("ffbd_gif_badsize.gif");
        let path = path.to_string_lossy().to_string();
        let mut writer = GifWriter::create(&path, 4, 4, 5).expect("create");
        let err = writer.add_frame(&[0u8; 5]).expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
