//! Where a model of whisper.cpp is looked for.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The model used where none is named. Kept the same as `DEFAULT_MODEL` in
/// `crates/speech/scripts/fetch-model.sh`, which fetches it.
pub const DEFAULT_MODEL: &str = "large-v3-turbo-q5_0";

/// The model of voice activity used where none is named
/// ([`crate::judge`]). Kept the same as the name the script fetches it by.
pub const DEFAULT_VAD_MODEL: &str = "silero-v5.1.2";

/// The directory `mise run speech-model` puts the models in:
/// `$XDG_CACHE_HOME/xmoq/whisper`, or `~/.cache/xmoq/whisper`. `None`
/// where neither variable is set.
///
/// WHY the same place on macOS and Linux, and not `~/Library/Caches`:
/// the script that fetches them is one shell script, and the place is one
/// that a person can say from memory.
#[must_use]
pub fn default_model_dir(
    xdg_cache_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let base = match xdg_cache_home.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home.filter(|dir| !dir.is_empty())?).join(".cache"),
    };
    Some(base.join("xmoq").join("whisper"))
}

/// The file of the model `name` (`tiny`, `large-v3-turbo-q5_0`) in `dir`.
#[must_use]
pub fn model_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("ggml-{name}.bin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_directory() {
        let cases = [
            (Some("/x"), Some("/home/a"), Some("/x/xmoq/whisper")),
            (None, Some("/home/a"), Some("/home/a/.cache/xmoq/whisper")),
            (
                Some(""),
                Some("/home/a"),
                Some("/home/a/.cache/xmoq/whisper"),
            ),
            (None, None, None),
        ];
        for (xdg, home, expected) in cases {
            assert_eq!(
                default_model_dir(xdg.map(OsString::from), home.map(OsString::from)),
                expected.map(PathBuf::from),
                "{xdg:?} {home:?}"
            );
        }
    }
}
