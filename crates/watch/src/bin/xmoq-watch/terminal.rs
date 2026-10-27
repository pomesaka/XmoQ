//! The terminal itself: its line mode, its keys, and the size of its window.
//!
//! WHY `rustix`: the workspace forbids `unsafe`, and `termios` and the
//! window's size are system calls, which `rustix` has as safe functions.
//! WHY NOT `crossterm`: it would do as well, and brings its own event loop
//! and screen handling for the two calls that are needed; it was not tried.

use std::io::Read;
use std::sync::{Mutex, PoisonError};

use rustix::termios::{
    LocalModes, OptionalActions, SpecialCodeIndex, Termios, tcgetattr, tcgetwinsize, tcsetattr,
};
use tokio::sync::mpsc;
use xmoq_watch::fit::cells;

/// What the terminal was set to, for whoever has to put it back.
static BEFORE: Mutex<Option<Termios>> = Mutex::new(None);

/// Puts the terminal back as it was. Does nothing the second time.
fn restore() {
    let before = BEFORE.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(termios) = before {
        // Nothing to do about a terminal that is gone.
        let _ = tcsetattr(std::io::stdin(), OptionalActions::Now, &termios);
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
/// starts reading it: one byte for a key on the channel. `None` where stdin
/// is no terminal.
///
/// WHY not all of raw mode: the output keeps its processing (a newline
/// still goes to the first column), since the picture is written with it.
/// WHY signals off (ISIG): Ctrl-C then comes as a byte like any key, and
/// the end goes one way, through the code that puts the terminal back.
/// A panic puts it back as well, by a hook; a signal that cannot be caught
/// does not.
pub fn keys() -> Option<(Raw, mpsc::Receiver<u8>)> {
    let mut termios = tcgetattr(std::io::stdin()).ok()?;
    *BEFORE.lock().unwrap_or_else(PoisonError::into_inner) = Some(termios.clone());
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    termios
        .local_modes
        .remove(LocalModes::ICANON | LocalModes::ECHO | LocalModes::ISIG);
    termios.special_codes[SpecialCodeIndex::VMIN] = 1;
    termios.special_codes[SpecialCodeIndex::VTIME] = 0;
    let raw = Raw;
    tcsetattr(std::io::stdin(), OptionalActions::Now, &termios).ok()?;
    let (keys, pressed) = mpsc::channel(64);
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        while matches!(std::io::stdin().read(&mut byte), Ok(1)) {
            let [byte] = byte;
            if keys.blocking_send(byte).is_err() {
                return;
            }
        }
    });
    Some((raw, pressed))
}

/// What the terminal on stdout says of itself: the most pixels a picture
/// with `lines` lines under it can have, and the pixels of a cell. `None`
/// where stdout is no terminal, or it does not say its pixels.
pub fn window(lines: usize) -> Option<((usize, usize), (usize, usize))> {
    let size = tcgetwinsize(std::io::stdout()).ok()?;
    let (columns, rows) = (usize::from(size.ws_col), usize::from(size.ws_row));
    let (width, height) = (usize::from(size.ws_xpixel), usize::from(size.ws_ypixel));
    if columns == 0 || rows == 0 || width == 0 || height == 0 {
        return None;
    }
    let cell = ((width / columns).max(1), (height / rows).max(1));
    // The lines under the picture, and one for the prompt the viewer was
    // started from.
    let most = (
        width,
        height.saturating_sub((lines + 1) * cell.1).max(cell.1),
    );
    debug_assert!(cells(most, 1, cell).1 <= rows);
    Some((most, cell))
}
