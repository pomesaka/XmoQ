//! A picture written as PNG, with the crate `png`. The one file that names
//! that crate.
//!
//! WHY `png`: Rust alone (no C compiler for it), the encoder of the `image`
//! crates without the rest of them.

/// A picture could not be written as PNG.
#[derive(Debug)]
pub struct NotWritten(String);

impl std::fmt::Display for NotWritten {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the picture as PNG: {}", self.0)
    }
}

impl std::error::Error for NotWritten {}

/// A picture of 3 bytes a pixel (red, green, blue) as the bytes of a PNG
/// file, 8 bits a channel.
///
/// # Errors
///
/// [`NotWritten`] for a size of nothing or beyond 32 bits, and for bytes
/// that are not as many as the size has.
pub fn png(rgb: &[u8], size: (usize, usize)) -> Result<Vec<u8>, NotWritten> {
    let length = |pixels: usize| {
        u32::try_from(pixels).map_err(|_| NotWritten(format!("a side of {pixels} pixels")))
    };
    let (width, height) = (length(size.0)?, length(size.1)?);
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| NotWritten(error.to_string()))?;
    writer
        .write_image_data(rgb)
        .map_err(|error| NotWritten(error.to_string()))?;
    writer
        .finish()
        .map_err(|error| NotWritten(error.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size and the pixels of a PNG file, read back with the same crate.
    fn read(file: &[u8]) -> ((u32, u32), Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(file))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(
            (info.color_type, info.bit_depth),
            (png::ColorType::Rgb, png::BitDepth::Eight)
        );
        pixels.truncate(info.buffer_size());
        ((info.width, info.height), pixels)
    }

    #[test]
    fn a_picture_comes_back_from_its_png_as_it_was() {
        let pixels = [
            255, 0, 0, 0, 255, 0, 0, 0, 255, //
            0, 0, 0, 128, 128, 128, 255, 255, 255,
        ];
        let file = png(&pixels, (3, 2)).unwrap();
        assert_eq!(file.get(..8), Some(&b"\x89PNG\r\n\x1a\n"[..]));
        assert_eq!(read(&file), ((3, 2), pixels.to_vec()));
    }

    #[test]
    fn a_picture_whose_bytes_do_not_fit_its_size_is_not_written() {
        // (the bytes; the size)
        let cases: [(&[u8], (usize, usize)); 4] = [
            (&[0; 17], (3, 2)),
            (&[0; 19], (3, 2)),
            (&[], (0, 0)),
            (&[0; 3], (1 << 33, 1)),
        ];
        for (bytes, size) in cases {
            assert!(png(bytes, size).is_err(), "{} bytes, {size:?}", bytes.len());
        }
    }
}
