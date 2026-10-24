//! What is written to a terminal: a picture with the Kitty graphics
//! protocol, and lines under it. One image that every frame replaces.
//!
//! The picture is put where the cursor was saved (`ESC 7`) when the room
//! for it was made, and every frame begins by going back there (`ESC 8`):
//! whatever else is written, the next frame is at the same place.

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The most bytes of base64 in one escape code (the protocol's limit for a
/// chunk of a payload).
const CHUNK: usize = 4096;

/// Appends `data` as base64, with padding.
pub fn base64(data: &[u8], out: &mut Vec<u8>) {
    let digit = |n: u32| {
        BASE64
            .get(usize::try_from(n & 63).unwrap_or(0))
            .copied()
            .unwrap_or(b'A')
    };
    for chunk in data.chunks(3) {
        let byte = |at: usize| u32::from(chunk.get(at).copied().unwrap_or(0));
        let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        out.push(digit(n >> 18));
        out.push(digit(n >> 12));
        out.push(if chunk.len() > 1 { digit(n >> 6) } else { b'=' });
        out.push(if chunk.len() > 2 { digit(n) } else { b'=' });
    }
}

/// Makes room for a picture of `rows` rows and `lines` lines under it, and
/// saves the cursor at its upper left.
///
/// WHY newlines and then back up: at the bottom of the screen the newlines
/// scroll, and the room is there; going down without them would not.
pub fn reserve(rows: usize, lines: usize, out: &mut Vec<u8>) {
    let all = rows + lines;
    out.extend(std::iter::repeat_n(b'\n', all));
    out.extend_from_slice(format!("\x1b[{all}A\x1b7").as_bytes());
}

/// The keys of the escape code of a frame: 24 bits a pixel, the image and
/// its placement both number 1 (so that a frame replaces the one before),
/// no answer from the terminal, the cursor left where it is.
fn keys(size: (usize, usize), cells: (usize, usize)) -> String {
    format!(
        "a=T,f=24,s={},v={},i=1,p=1,q=2,C=1,c={},r={}",
        size.0, size.1, cells.0, cells.1
    )
}

/// A frame with its pixels in the escape codes: for a terminal anywhere.
pub fn frame_direct(rgb: &[u8], size: (usize, usize), cells: (usize, usize), out: &mut Vec<u8>) {
    let mut encoded = Vec::with_capacity(rgb.len().div_ceil(3) * 4);
    base64(rgb, &mut encoded);
    out.extend_from_slice(b"\x1b8");
    let keys = keys(size, cells);
    let mut chunks = encoded.chunks(CHUNK).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            out.extend_from_slice(format!("\x1b_G{keys},m={more};").as_bytes());
            first = false;
        } else {
            out.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// A frame whose pixels are in a file the terminal reads and removes
/// (`t=t`): for a terminal on the same machine. The terminal removes the
/// file only where its name has `tty-graphics-protocol` in it.
pub fn frame_file(path: &str, size: (usize, usize), cells: (usize, usize), out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b8");
    out.extend_from_slice(format!("\x1b_G{},t=t;", keys(size, cells)).as_bytes());
    base64(path.as_bytes(), out);
    out.extend_from_slice(b"\x1b\\");
}

/// The lines under a picture of `rows` rows, written anew. The lines are
/// to be without control characters ([`xmoq_viewer::room::clean`]).
pub fn lines_under<'a>(rows: usize, lines: impl IntoIterator<Item = &'a str>, out: &mut Vec<u8>) {
    out.extend_from_slice(format!("\x1b8\x1b[{rows}B").as_bytes());
    for line in lines {
        // To the first column, the line cleared, the text, and one down
        // without scrolling.
        out.extend_from_slice(format!("\r\x1b[K{line}\x1b[B").as_bytes());
    }
}

/// Leaves the cursor under the picture and its lines.
pub fn leave(rows: usize, lines: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(format!("\x1b8\x1b[{}B\n", rows + lines).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(write: impl FnOnce(&mut Vec<u8>)) -> String {
        let mut out = Vec::new();
        write(&mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn base64_is_that_of_rfc_4648() {
        // The test vectors of RFC 4648, section 10.
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in cases {
            assert_eq!(
                text(|out| base64(plain.as_bytes(), out)),
                encoded,
                "{plain}"
            );
        }
        assert_eq!(text(|out| base64(&[0xff, 0xff, 0xfe], out)), "///+");
    }

    #[test]
    fn a_small_frame_is_one_escape_code() {
        // One red pixel and one blue.
        let frame = text(|out| frame_direct(&[255, 0, 0, 0, 0, 255], (2, 1), (1, 1), out));
        assert_eq!(
            frame,
            "\x1b8\x1b_Ga=T,f=24,s=2,v=1,i=1,p=1,q=2,C=1,c=1,r=1,m=0;/wAAAAD/\x1b\\"
        );
    }

    #[test]
    fn a_frame_of_more_than_a_chunk_is_cut_and_only_the_last_says_so() {
        // 3,072 pixels are 9,216 bytes and 12,288 of base64: three chunks.
        let frame = text(|out| frame_direct(&vec![0; 9_216], (3_072, 1), (384, 1), out));
        let codes: Vec<(&str, &str)> = frame
            .split_terminator("\x1b\\")
            .map(|code| code.split_once(';').unwrap())
            .collect();
        let heads: Vec<&str> = codes.iter().map(|(head, _)| *head).collect();
        let lengths: Vec<usize> = codes.iter().map(|(_, payload)| payload.len()).collect();
        assert_eq!(
            heads,
            [
                "\x1b8\x1b_Ga=T,f=24,s=3072,v=1,i=1,p=1,q=2,C=1,c=384,r=1,m=1",
                "\x1b_Gm=1",
                "\x1b_Gm=0",
            ]
        );
        assert_eq!(lengths, [4_096, 4_096, 4_096]);
    }

    #[test]
    fn a_frame_in_a_file_names_the_file() {
        let frame = text(|out| frame_file("/tmp/a.rgb", (640, 360), (80, 23), out));
        assert_eq!(
            frame,
            "\x1b8\x1b_Ga=T,f=24,s=640,v=360,i=1,p=1,q=2,C=1,c=80,r=23,t=t;L3RtcC9hLnJnYg==\x1b\\"
        );
    }

    #[test]
    fn room_is_made_and_left_and_lines_are_written_under_the_picture() {
        assert_eq!(text(|out| reserve(2, 1, out)), "\n\n\n\x1b[3A\x1b7");
        assert_eq!(text(|out| leave(2, 1, out)), "\x1b8\x1b[3B\n");
        assert_eq!(
            text(|out| lines_under(23, ["1 a: hi", "[q] quit"], out)),
            "\x1b8\x1b[23B\r\x1b[K1 a: hi\x1b[B\r\x1b[K[q] quit\x1b[B"
        );
        assert_eq!(text(|out| lines_under(23, [], out)), "\x1b8\x1b[23B");
    }
}
