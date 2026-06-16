//! Pure pixel → cell downscaling: split a grayscale frame into a `cols × rows` grid and
//! box-average each region down to an 8×8 [`veyo_core::Cell`]. No image-format
//! dependency — [`crate::decode`] turns PNG bytes into the grayscale buffer this
//! consumes, so this logic is testable on synthetic buffers.

use veyo_core::{Cell, CELL_LEN, CELL_SIDE};

/// Split a `w × h` grayscale buffer into `cols × rows` region cells (row-major), each
/// box-averaged down to 8×8. `gray.len()` must be `w * h`.
pub fn gray_to_cells(gray: &[u8], w: usize, h: usize, cols: usize, rows: usize) -> Vec<Cell> {
    debug_assert_eq!(gray.len(), w * h, "grayscale buffer length must be w * h");
    let cols = cols.max(1);
    let rows = rows.max(1);
    let mut cells = Vec::with_capacity(cols * rows);
    for ry in 0..rows {
        for cx in 0..cols {
            let x0 = cx * w / cols;
            let x1 = ((cx + 1) * w / cols).max(x0 + 1).min(w);
            let y0 = ry * h / rows;
            let y1 = ((ry + 1) * h / rows).max(y0 + 1).min(h);
            cells.push(downsample_region(gray, w, x0, x1, y0, y1));
        }
    }
    cells
}

fn downsample_region(gray: &[u8], w: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> Cell {
    let rw = x1 - x0;
    let rh = y1 - y0;
    let mut cell = [0u8; CELL_LEN];
    for by in 0..CELL_SIDE {
        for bx in 0..CELL_SIDE {
            let sx0 = x0 + bx * rw / CELL_SIDE;
            let sx1 = (x0 + (bx + 1) * rw / CELL_SIDE).max(sx0 + 1).min(x1);
            let sy0 = y0 + by * rh / CELL_SIDE;
            let sy1 = (y0 + (by + 1) * rh / CELL_SIDE).max(sy0 + 1).min(y1);
            let mut sum = 0u32;
            let mut count = 0u32;
            for yy in sy0..sy1 {
                for xx in sx0..sx1 {
                    sum += gray[yy * w + xx] as u32;
                    count += 1;
                }
            }
            cell[by * CELL_SIDE + bx] = (sum / count.max(1)) as u8;
        }
    }
    cell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_image_yields_uniform_cells() {
        let gray = vec![120u8; 16 * 16];
        let cells = gray_to_cells(&gray, 16, 16, 1, 1);
        assert_eq!(cells.len(), 1);
        assert!(cells[0].iter().all(|&p| p == 120), "got {:?}", cells[0]);
    }

    #[test]
    fn left_black_right_white_splits_into_two_cells() {
        // 16 wide × 8 tall; left half (x<8) = 0, right half = 255.
        let (w, h) = (16usize, 8usize);
        let mut gray = vec![0u8; w * h];
        for y in 0..h {
            for x in 8..w {
                gray[y * w + x] = 255;
            }
        }
        let cells = gray_to_cells(&gray, w, h, 2, 1);
        assert_eq!(cells.len(), 2);
        assert!(cells[0].iter().all(|&p| p == 0), "left cell {:?}", cells[0]);
        assert!(
            cells[1].iter().all(|&p| p == 255),
            "right cell {:?}",
            cells[1]
        );
    }

    #[test]
    fn cell_count_matches_grid() {
        let gray = vec![50u8; 32 * 32];
        assert_eq!(gray_to_cells(&gray, 32, 32, 4, 3).len(), 12);
    }

    #[test]
    fn non_divisible_dimensions_do_not_panic() {
        // 17×13 into an 8×8 grid: nothing divides evenly.
        let gray = vec![100u8; 17 * 13];
        let cells = gray_to_cells(&gray, 17, 13, 8, 8);
        assert_eq!(cells.len(), 64);
        assert!(cells.iter().all(|c| c.iter().all(|&p| p == 100)));
    }

    #[test]
    fn more_columns_than_pixels_is_safe() {
        // 4px wide into 8 columns: some regions are 1px wide — must not panic / div0.
        let gray = vec![77u8; 4 * 2];
        let cells = gray_to_cells(&gray, 4, 2, 8, 1);
        assert_eq!(cells.len(), 8);
        assert!(cells.iter().all(|c| c.iter().all(|&p| p == 77)));
    }

    #[test]
    fn horizontal_gradient_orders_left_to_right() {
        let (w, h) = (12usize, 1usize);
        let gray: Vec<u8> = (0..w).map(|x| (x * 255 / (w - 1)) as u8).collect();
        let cells = gray_to_cells(&gray, w, h, 3, 1);
        let mean = |c: &Cell| c.iter().map(|&p| p as u32).sum::<u32>() / 64;
        assert!(
            mean(&cells[0]) < mean(&cells[2]),
            "left {} !< right {}",
            mean(&cells[0]),
            mean(&cells[2])
        );
    }
}
