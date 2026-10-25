//! Which reader reads the pieces, and opening it: the one place that names
//! the implementations of [`Words`], so that a transcriber knows the trait
//! alone and a reader is added here and in its own file.
//!
//! WHY a choice of readers (asked for on 2026-10-25 by the project owner):
//! the transcription is an experiment, and what reads the words may become
//! another program than whisper.cpp, an agent among them. How the sound is
//! cut, and what is done with the words, stay outside the reader.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::Words;
use crate::command::{Command, CommandLimits};
use crate::model::{DEFAULT_MODEL, model_file};
use crate::whisper::Whisper;

/// A reader of pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// whisper.cpp, with a model of its own ([`crate::model`]).
    Whisper,
    /// A program run for each piece ([`crate::command`]).
    Command,
}

impl Engine {
    /// The names a flag gives them by.
    pub const NAMES: &str = "whisper, command";

    /// The reader named `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "whisper" => Some(Self::Whisper),
            "command" => Some(Self::Command),
            _ => None,
        }
    }
}

/// What a reader is opened with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineOptions {
    /// The file of the model; `None` for the default in `cache`.
    pub model: Option<PathBuf>,
    /// The directory the models are fetched into
    /// ([`crate::model::default_model_dir`]), where it is known.
    pub cache: Option<PathBuf>,
    /// A language code of whisper's (`ja`, `en`), or `auto`.
    pub language: String,
    /// For [`Engine::Command`]: the program and its arguments.
    pub command: Vec<String>,
    /// For [`Engine::Command`]: what the program is given before it is
    /// stopped.
    pub command_limits: CommandLimits,
}

/// The readers that were opened, and what to say of them.
pub struct Opened {
    /// One for each thread that reads.
    pub readers: Vec<Box<dyn Words + Send>>,
    /// What was opened, as fields of a line (`path=... language=...`).
    pub said: String,
    /// The directory the readers write to, removed when this is dropped;
    /// `None` where they write nothing.
    pub dir: Option<Scratch>,
}

/// A directory of the readers' own, removed with what is in it when dropped.
///
/// WHY on drop: the transcriber returns from its loop on SIGTERM and Ctrl-C
/// as on an error, so a value held there is dropped however it ends, short
/// of SIGKILL.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opens `count` readers of `engine`, at least one: they read at once, each
/// on a thread of its own.
///
/// # Errors
/// When the reader cannot be opened: its model is missing, say.
pub fn open(engine: Engine, options: &EngineOptions, count: usize) -> Result<Opened> {
    match engine {
        Engine::Whisper => {
            let model = match (&options.model, &options.cache) {
                (Some(model), _) => model.clone(),
                (None, Some(cache)) => model_file(cache, DEFAULT_MODEL),
                (None, None) => {
                    anyhow::bail!("neither XDG_CACHE_HOME nor HOME is set: give the model's file")
                }
            };
            let first = Whisper::open(&model, &options.language)
                .context("mise run speech-model fetches the default model")?;
            let mut readers: Vec<Box<dyn Words + Send>> = Vec::with_capacity(count);
            for _ in 1..count {
                readers.push(Box::new(first.another()?));
            }
            readers.push(Box::new(first));
            Ok(Opened {
                readers,
                said: format!(
                    "engine=whisper path={} language={}",
                    model.display(),
                    options.language
                ),
                dir: None,
            })
        }
        Engine::Command => {
            // WHY a directory of this process's own: two transcribers on one
            // machine write pieces of the same names.
            let dir = std::env::temp_dir().join(format!("xmoq-transcriber-{}", std::process::id()));
            std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
            let dir = Scratch(dir);
            let mut readers: Vec<Box<dyn Words + Send>> = Vec::with_capacity(count);
            for n in 0..count.max(1) {
                readers.push(Box::new(Command::new(
                    options.command.clone(),
                    &options.language,
                    options.command_limits,
                    dir.path().join(format!("piece-{n}.wav")),
                )?));
            }
            Ok(Opened {
                readers,
                said: format!(
                    "engine=command command={} language={}",
                    options.command.join(" "),
                    options.language
                ),
                dir: Some(dir),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_are_named_by_their_flag() {
        let cases: [(&str, Option<Engine>); 3] = [
            ("whisper", Some(Engine::Whisper)),
            ("command", Some(Engine::Command)),
            ("Whisper", None),
        ];
        for (name, expected) in cases {
            assert_eq!(Engine::named(name), expected, "{name}");
        }
    }

    #[test]
    fn the_directory_of_command_readers_goes_when_dropped() {
        let options = EngineOptions {
            model: None,
            cache: None,
            language: "en".to_owned(),
            command: vec!["true".to_owned()],
            command_limits: CommandLimits::default(),
        };
        let opened = open(Engine::Command, &options, 2).unwrap();
        let dir = opened.dir.as_ref().unwrap().path().to_owned();
        assert!(dir.is_dir());
        drop(opened);
        assert!(!dir.exists());
    }
}
