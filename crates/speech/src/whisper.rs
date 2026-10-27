//! whisper.cpp, through the crate `whisper-rs`: the one file that uses it,
//! so that it can be changed here alone.
//!
//! WHY `whisper-rs` 0.16.0: it builds whisper.cpp 1.8.3 from the source it
//! carries, has a safe API over it (the workspace forbids `unsafe`), and
//! was the one that crates.io listed with releases through 2026
//! (repository on codeberg.org, read on 2026-10-25). WHY NOT the
//! program `whisper-cli` of whisper.cpp, run for each piece: the model
//! would be loaded again for every piece, seconds for the large ones.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};

use crate::Words;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// A model of whisper.cpp, loaded, with the state a piece is read in.
pub struct Whisper {
    /// The model, which [`Self::another`] shares.
    context: Arc<WhisperContext>,
    state: WhisperState,
    language: Option<String>,
}

impl Whisper {
    /// Loads the model at `model`. `language` is a code of whisper's
    /// (`ja`, `en`), or `auto` to have each piece's language found.
    ///
    /// # Errors
    /// When the model cannot be read or loaded.
    pub fn open(model: &Path, language: &str) -> Result<Self> {
        // whisper.cpp and ggml write their progress to stderr unless this is
        // called; without the features `log_backend` or `tracing_backend` it
        // drops it.
        whisper_rs::install_logging_hooks();
        let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
            .map_err(|error| anyhow!("{error}"))
            .with_context(|| format!("loading the model {}", model.display()))?;
        let state = context
            .create_state()
            .map_err(|error| anyhow!("the state of whisper.cpp: {error}"))?;
        Ok(Self {
            context: Arc::new(context),
            state,
            language: (language != "auto").then(|| language.to_owned()),
        })
    }

    /// Another reader of the same model, in the same language, with a state
    /// of its own: it reads on another thread while this one reads, and the
    /// model is held once (`whisper_init_state` of whisper.cpp, which
    /// `create_state` of `whisper-rs` 0.16.0 calls, gives a state that
    /// refers to the model).
    ///
    /// # Errors
    /// When whisper.cpp makes no state.
    pub fn another(&self) -> Result<Self> {
        let state = self
            .context
            .create_state()
            .map_err(|error| anyhow!("the state of whisper.cpp: {error}"))?;
        Ok(Self {
            context: Arc::clone(&self.context),
            state,
            language: self.language.clone(),
        })
    }

    /// The words of `samples` (one channel at 16 kHz), trimmed; empty where
    /// whisper finds none.
    ///
    /// # Errors
    /// When whisper.cpp fails.
    pub fn text(&mut self, samples: &[f32]) -> Result<String> {
        // WHY greedy and not a beam: a beam of 5 takes several times as long,
        // and a live line that comes late is worth less than a word it got
        // wrong.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(self.language.as_deref().unwrap_or("auto")));
        // Each piece is read on its own: the words of the piece before, given
        // as context, are what whisper repeats when it makes words up.
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_no_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        self.state
            .full(params, samples)
            .map_err(|error| anyhow!("whisper.cpp: {error}"))?;
        let mut text = String::new();
        for segment in self.state.as_iter() {
            let words = segment
                .to_str_lossy()
                .map_err(|error| anyhow!("whisper.cpp: {error}"))?;
            text.push_str(&words);
        }
        Ok(text.trim().to_owned())
    }
}

impl Words for Whisper {
    fn text(&mut self, samples: &[f32]) -> Result<String> {
        Whisper::text(self, samples)
    }
}
