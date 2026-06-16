//! PNG → grayscale frame decoding (feature `decode`).
//!
//! Phase-0 recording format is a PNG frame dump; this turns each PNG into the grayscale
//! buffer [`crate::downscale::gray_to_cells`] consumes. Video decode (ffmpeg) is out of
//! Phase-0 scope.

use crate::downscale::gray_to_cells;
use anyhow::{bail, ensure, Context, Result};
use std::path::Path;
use veyo_core::Cell;

/// Decode an 8-bit PNG file to a grayscale buffer of length `w * h`.
pub fn decode_png_gray(path: &Path) -> Result<(Vec<u8>, usize, usize)> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .with_context(|| format!("read PNG header {}", path.display()))?;
    let buf_size = reader
        .output_buffer_size()
        .context("PNG dimensions too large for an output buffer")?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf).context("decode PNG frame")?;
    let (w, h) = (info.width as usize, info.height as usize);
    let gray = to_gray(
        &buf[..info.buffer_size()],
        info.color_type,
        info.bit_depth,
        w,
        h,
    )?;
    Ok((gray, w, h))
}

/// Decode a PNG straight into `cols × rows` region cells plus its `(w, h)` dimensions.
pub fn load_cells(path: &Path, cols: usize, rows: usize) -> Result<(Vec<Cell>, (u32, u32))> {
    let (gray, w, h) = decode_png_gray(path)?;
    Ok((gray_to_cells(&gray, w, h, cols, rows), (w as u32, h as u32)))
}

fn to_gray(
    bytes: &[u8],
    color: png::ColorType,
    depth: png::BitDepth,
    w: usize,
    h: usize,
) -> Result<Vec<u8>> {
    ensure!(
        depth == png::BitDepth::Eight,
        "only 8-bit PNG frames are supported"
    );
    let px = w * h;
    let gray = match color {
        png::ColorType::Grayscale => bytes[..px].to_vec(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).take(px).map(|c| c[0]).collect(),
        png::ColorType::Rgb => bytes
            .chunks_exact(3)
            .take(px)
            .map(|c| luma(c[0], c[1], c[2]))
            .collect(),
        png::ColorType::Rgba => bytes
            .chunks_exact(4)
            .take(px)
            .map(|c| luma(c[0], c[1], c[2]))
            .collect(),
        png::ColorType::Indexed => bail!("indexed PNG unsupported; re-export as RGB or grayscale"),
    };
    Ok(gray)
}

/// Rec. 601 luma from RGB.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_matches_rec601() {
        assert_eq!(luma(255, 255, 255), 255);
        assert_eq!(luma(0, 0, 0), 0);
        assert_eq!(luma(255, 0, 0), 76);
        assert_eq!(luma(0, 255, 0), 149);
        assert_eq!(luma(0, 0, 255), 29);
    }

    #[test]
    fn to_gray_handles_grayscale_and_alpha() {
        let g = to_gray(
            &[1, 2, 3, 4],
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            2,
            2,
        )
        .unwrap();
        assert_eq!(g, vec![1, 2, 3, 4]);
        let ga = to_gray(
            &[10, 255, 20, 255, 30, 255, 40, 255],
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            2,
            2,
        )
        .unwrap();
        assert_eq!(ga, vec![10, 20, 30, 40]);
    }

    #[test]
    fn to_gray_converts_rgb_and_rgba_via_luma() {
        let rgb = to_gray(
            &[255, 0, 0],
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            1,
            1,
        )
        .unwrap();
        assert_eq!(rgb, vec![76]);
        let rgba = to_gray(
            &[0, 255, 0, 128],
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            1,
            1,
        )
        .unwrap();
        assert_eq!(rgba, vec![149]);
    }

    #[test]
    fn to_gray_rejects_indexed_and_non_8bit() {
        assert!(to_gray(&[0], png::ColorType::Indexed, png::BitDepth::Eight, 1, 1).is_err());
        assert!(to_gray(
            &[0, 0],
            png::ColorType::Grayscale,
            png::BitDepth::Sixteen,
            1,
            1
        )
        .is_err());
    }
}
