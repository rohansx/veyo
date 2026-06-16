use veyo_core::{Cell, CELL_LEN, CELL_SIDE};

/// Convert an RGBA frame into a grid of 8×8 luminance cells.
///
/// Each cell covers a `(w/cols) × (h/rows)` region of the source frame and
/// is box-averaged down to `CELL_SIDE × CELL_SIDE` luma samples.
///
/// Luma: Rec.601 `Y = (299·R + 587·G + 114·B) / 1000`
pub fn rgba_to_cells(rgba: &[u8], w: u32, h: u32, cols: u8, rows: u8) -> Vec<Cell> {
    debug_assert_eq!(rgba.len(), (w * h * 4) as usize);
    let cols = cols.max(1) as u32;
    let rows = rows.max(1) as u32;
    let cw = (w / cols) as usize;
    let ch = (h / rows) as usize;
    let w = w as usize;

    let mut out = Vec::with_capacity((cols * rows) as usize);
    for row in 0..rows as usize {
        for col in 0..cols as usize {
            let x0 = col * cw;
            let y0 = row * ch;
            out.push(region_to_cell(rgba, w, x0, y0, cw, ch));
        }
    }
    out
}

fn region_to_cell(rgba: &[u8], stride: usize, x0: usize, y0: usize, rw: usize, rh: usize) -> Cell {
    let mut cell = [0u8; CELL_LEN];
    let bw = (rw / CELL_SIDE).max(1);
    let bh = (rh / CELL_SIDE).max(1);

    for cy in 0..CELL_SIDE {
        for cx in 0..CELL_SIDE {
            let px0 = x0 + cx * bw;
            let py0 = y0 + cy * bh;
            let mut sum: u32 = 0;
            let mut count: u32 = 0;
            for dy in 0..bh {
                for dx in 0..bw {
                    let pi = ((py0 + dy) * stride + (px0 + dx)) * 4;
                    if pi + 3 < rgba.len() {
                        let r = rgba[pi] as u32;
                        let g = rgba[pi + 1] as u32;
                        let b = rgba[pi + 2] as u32;
                        sum += (299 * r + 587 * g + 114 * b) / 1000;
                        count += 1;
                    }
                }
            }
            cell[cy * CELL_SIDE + cx] = sum.checked_div(count).unwrap_or(0) as u8;
        }
    }
    cell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_white_frame_produces_all_255_cells() {
        let w: u32 = 64;
        let h: u32 = 64;
        let rgba = vec![255u8; (w * h * 4) as usize];
        let cells = rgba_to_cells(&rgba, w, h, 1, 1);
        assert_eq!(cells.len(), 1);
        assert!(cells[0].iter().all(|&v| v == 255));
    }

    #[test]
    fn solid_black_frame_produces_all_zero_cells() {
        let w: u32 = 64;
        let h: u32 = 64;
        let rgba = vec![0u8; (w * h * 4) as usize];
        let cells = rgba_to_cells(&rgba, w, h, 2, 2);
        assert_eq!(cells.len(), 4);
        assert!(cells.iter().all(|c| c.iter().all(|&v| v == 0)));
    }

    #[test]
    fn cell_count_matches_grid() {
        let rgba = vec![128u8; 1280 * 720 * 4];
        let cells = rgba_to_cells(&rgba, 1280, 720, 8, 8);
        assert_eq!(cells.len(), 64);
    }

    #[test]
    fn luma_conversion_red_is_darker_than_green() {
        // Pure red: Y ≈ 0.299, pure green: Y ≈ 0.587
        let make_solid = |r: u8, g: u8, b: u8| -> Vec<u8> {
            let px = vec![r, g, b, 255];
            px.repeat(64 * 64)
        };
        let red_cells = rgba_to_cells(&make_solid(255, 0, 0), 64, 64, 1, 1);
        let grn_cells = rgba_to_cells(&make_solid(0, 255, 0), 64, 64, 1, 1);
        assert!(
            red_cells[0][0] < grn_cells[0][0],
            "red luma {} should be < green luma {}",
            red_cells[0][0],
            grn_cells[0][0]
        );
    }
}
