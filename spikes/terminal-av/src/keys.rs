//! The keyboard while watching: the terminal is taken out of line mode, so
//! that a key comes when it is pressed, and put back when the program ends.

use std::io::Read;
use std::sync::Mutex;

use tokio::sync::mpsc;

/// What the terminal was set to, for whoever has to put it back.
static BEFORE: Mutex<Option<libc::termios>> = Mutex::new(None);

/// Puts the terminal back as it was. Safe to call more than once.
fn restore() {
    if let Ok(mut before) = BEFORE.lock()
        && let Some(termios) = before.take()
    {
        // SAFETY: file descriptor 0 and a termios that tcgetattr filled.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) };
    }
}

/// The terminal is as it was again when this is dropped.
pub struct Raw;

impl Drop for Raw {
    fn drop(&mut self) {
        restore();
    }
}

/// Takes stdin out of line mode and echo, where it is a terminal, and
/// starts reading it: one byte for a key on the channel. None where stdin is
/// no terminal.
///
/// WHY not all of `cfmakeraw`: the output keeps its processing (a newline
/// still goes to the first column), since the picture is written with it.
/// WHY signals off (ISIG): Ctrl-C then comes as a byte like any key, and the
/// end goes one way, through the code that puts the terminal back.
/// A panic puts it back as well, by a hook; a signal that cannot be caught
/// does not.
pub fn start() -> Option<(Raw, mpsc::Receiver<u8>)> {
    // SAFETY: an all-zero termios is a valid value for tcgetattr to fill.
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: file descriptor 0 and a termios of this frame.
    if unsafe { libc::tcgetattr(0, &mut termios) } != 0 {
        return None;
    }
    *BEFORE.lock().ok()? = Some(termios);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    termios.c_cc[libc::VMIN] = 1;
    termios.c_cc[libc::VTIME] = 0;
    // SAFETY: as above.
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &termios) };
    let (keys, pressed) = mpsc::channel(64);
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        while matches!(std::io::stdin().read(&mut byte), Ok(1)) {
            if keys.blocking_send(byte[0]).is_err() {
                return;
            }
        }
    });
    Some((Raw, pressed))
}

/// What a key asks for.
pub enum Asked {
    Quit,
    Say(String),
    Louder,
    Quieter,
    Mute,
}

/// The keys of the viewer: `q` or Ctrl-C ends, `+` and `-` the volume, `m`
/// mutes, `t` or Enter begins a line for the chat, which Enter sends and
/// Esc gives up.
#[derive(Default)]
pub struct Keys {
    /// The line that is being typed, as bytes of UTF-8.
    typing: Option<Vec<u8>>,
}

impl Keys {
    pub fn press(&mut self, byte: u8) -> Option<Asked> {
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
            // Backspace takes a whole character: its last byte and the bytes that continue it.
            0x7f | 0x08 => {
                while let Some(last) = line.pop() {
                    if last & 0xc0 != 0x80 {
                        break;
                    }
                }
                None
            }
            byte if byte >= 0x20 && line.len() < 400 => {
                line.push(byte);
                None
            }
            _ => None,
        }
    }

    /// The line being typed, for showing it.
    pub fn typing(&self) -> Option<String> {
        self.typing.as_ref().map(|line| String::from_utf8_lossy(line).into_owned())
    }
}
