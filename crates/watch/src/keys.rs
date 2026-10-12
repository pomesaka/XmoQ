//! What a key asks for.

/// What the viewer is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// End.
    Quit,
    /// Send this line to the chat.
    Say(String),
    /// Turn the sound up.
    Louder,
    /// Turn the sound down.
    Quieter,
    /// Mute the sound, or end the mute.
    Mute,
}

/// The most bytes of a line that is typed.
const LINE_LIMIT: usize = 400;

/// The keys of the viewer, as the bytes a terminal out of line mode gives:
/// `q` or Ctrl-C ends, `+` (or `=`) and `-` change the volume, `m` mutes,
/// `t` or Enter begins a line for the chat, which Enter sends and Esc gives
/// up, and Backspace takes a character of.
#[derive(Debug, Default)]
pub struct Keys {
    /// The line that is being typed, as bytes of UTF-8.
    typing: Option<Vec<u8>>,
}

impl Keys {
    /// Takes one byte, and says what it asks for, if anything.
    pub fn press(&mut self, byte: u8) -> Option<Asked> {
        // Ctrl-C ends whatever is being typed: a terminal out of line mode
        // sends it as a byte, and nothing else would end the viewer then.
        if byte == 0x03 {
            return Some(Asked::Quit);
        }
        let Some(line) = &mut self.typing else {
            return match byte {
                b'q' => Some(Asked::Quit),
                b'+' | b'=' => Some(Asked::Louder),
                b'-' => Some(Asked::Quieter),
                b'm' => Some(Asked::Mute),
                b't' | b'\r' | b'\n' => {
                    self.typing = Some(Vec::new());
                    None
                }
                _ => None,
            };
        };
        match byte {
            b'\r' | b'\n' => {
                let text = String::from_utf8_lossy(line).trim().to_string();
                self.typing = None;
                (!text.is_empty()).then_some(Asked::Say(text))
            }
            0x1b => {
                self.typing = None;
                None
            }
            // Backspace takes a whole character: its last byte and the
            // bytes that continue it.
            0x7f | 0x08 => {
                while let Some(last) = line.pop() {
                    if last & 0xc0 != 0x80 {
                        break;
                    }
                }
                None
            }
            byte if byte >= 0x20 && line.len() < LINE_LIMIT => {
                line.push(byte);
                None
            }
            _ => None,
        }
    }

    /// The line being typed, for showing it; `None` while none is.
    #[must_use]
    pub fn typing(&self) -> Option<String> {
        self.typing
            .as_ref()
            .map(|line| String::from_utf8_lossy(line).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(bytes: &[u8]) -> (Vec<Asked>, Option<String>) {
        let mut keys = Keys::default();
        let asked = bytes.iter().filter_map(|byte| keys.press(*byte)).collect();
        (asked, keys.typing())
    }

    #[test]
    fn keys_ask_for_what_they_stand_for() {
        let say = |text: &str| Asked::Say(text.to_string());
        // (the bytes; what was asked for, in order; the line still being typed)
        let cases: Vec<(&[u8], Vec<Asked>, Option<&str>)> = vec![
            (b"q", vec![Asked::Quit], None),
            (b"\x03", vec![Asked::Quit], None),
            (
                b"+=-m",
                vec![Asked::Louder, Asked::Louder, Asked::Quieter, Asked::Mute],
                None,
            ),
            // Keys that stand for nothing.
            (b"xyz \x1b\x7f", vec![], None),
            (b"thello\r", vec![say("hello")], None),
            (b"\rhi\n", vec![say("hi")], None),
            // While a line is typed, `q` and `m` are letters.
            (b"tqm+", vec![], Some("qm+")),
            (b"tabc\x7f\x7fd", vec![], Some("ad")),
            // Backspace takes the three bytes of a character at once.
            ("ta\u{3042}\x7fb".as_bytes(), vec![], Some("ab")),
            // More Backspaces than characters.
            (b"ta\x7f\x7f\x7f", vec![], Some("")),
            (b"tabc\x1b", vec![], None),
            // Esc gives the line up, and the next keys are keys again.
            (b"tabc\x1bq", vec![Asked::Quit], None),
            // A line of nothing but blanks is not sent.
            (b"t   \r", vec![], None),
            (b"t  a b  \r", vec![say("a b")], None),
            // Ctrl-C ends, also in the middle of a line.
            (b"tab\x03", vec![Asked::Quit], Some("ab")),
            // Control characters do not get into a line.
            (b"ta\x01\x09b", vec![], Some("ab")),
        ];
        for (bytes, asked, typing) in cases {
            assert_eq!(
                pressed(bytes),
                (asked, typing.map(str::to_string)),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn a_line_is_no_longer_than_its_limit() {
        let mut keys = Keys::default();
        keys.press(b't');
        for _ in 0..500 {
            keys.press(b'a');
        }
        assert_eq!(keys.typing().map(|line| line.len()), Some(400));
    }
}
