//! Making a picture fit a terminal. By how much it is made smaller is
//! `xmoq_picture::fit`'s.

/// The cells of a terminal a picture takes, at `scale` times its pixels,
/// where a cell has `cell` pixels.
#[must_use]
pub fn cells(size: (usize, usize), scale: usize, cell: (usize, usize)) -> (usize, usize) {
    let each = |pixels: usize, cell: usize| pixels.saturating_mul(scale).div_ceil(cell.max(1));
    (each(size.0, cell.0), each(size.1, cell.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_takes_the_cells_its_pixels_reach_into() {
        // (the picture; the scale; a cell; the cells)
        let cases = [
            ((640, 360), 1, (8, 16), (80, 23)),
            ((640, 360), 2, (8, 16), (160, 45)),
            ((960, 540), 1, (9, 18), (107, 30)),
            ((1, 1), 1, (8, 16), (1, 1)),
            ((0, 0), 1, (8, 16), (0, 0)),
            // A cell of nothing is taken for one pixel.
            ((10, 10), 1, (0, 0), (10, 10)),
        ];
        for (size, scale, cell, expected) in cases {
            assert_eq!(cells(size, scale, cell), expected, "{size:?}");
        }
    }
}
