//! The model of voice activity of whisper.cpp (Silero), through `whisper-rs`:
//! with [`crate::whisper`], the files that use it.

use std::path::Path;

use anyhow::{Context as _, Result, anyhow};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams};

use crate::judge::Probabilities;

/// A model of voice activity, loaded. One for each track: it is 885 KB.
pub struct Silero {
    context: WhisperVadContext,
}

impl Silero {
    /// Loads the model at `model` (`ggml-silero-v5.1.2.bin`).
    ///
    /// # Errors
    /// When the model cannot be read or loaded.
    pub fn open(model: &Path) -> Result<Self> {
        whisper_rs::install_logging_hooks();
        let path = model
            .to_str()
            .with_context(|| format!("{} is no UTF-8", model.display()))?;
        let mut params = WhisperVadContextParams::default();
        // WHY on the processor: the model is small, and 32 ms of sound took
        // 0.12 ms there; a graphics processor would compile its code first.
        params.set_use_gpu(false);
        params.set_n_threads(1);
        let context = WhisperVadContext::new(path, params)
            .map_err(|error| anyhow!("{error}"))
            .with_context(|| format!("loading the model of voice activity {}", model.display()))?;
        Ok(Self { context })
    }
}

impl Probabilities for Silero {
    fn probabilities(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        self.context
            .detect_speech(samples)
            .map_err(|error| anyhow!("the model of voice activity: {error}"))?;
        Ok(self.context.probabilities().to_vec())
    }
}
