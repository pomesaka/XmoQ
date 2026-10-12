//! The sound output of the machine, with the crate `cpal`. The one file
//! that names that crate; which output the viewer uses is not decided
//! (`docs/open-questions.md`).

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use xmoq_watch::playout::RATE;

/// The default output device, playing. Closed when it is dropped.
pub struct AudioOutput {
    _stream: cpal::Stream,
}

impl AudioOutput {
    /// Opens the default output device at 48 kHz, and has it ask `fill`
    /// for its samples: `fill` is given the moment the first sample of the
    /// call leaves the machine, and one channel to fill, which is played on
    /// every channel of the device.
    ///
    /// WHY 48 kHz or nothing: that is what Opus is decoded to, and nothing
    /// here resamples.
    pub fn open(mut fill: impl FnMut(Instant, &mut [f32]) + Send + 'static) -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow!("the machine has no sound output"))?;
        let config = device
            .supported_output_configs()?
            .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
            .find_map(|range| range.try_with_sample_rate(RATE))
            .ok_or_else(|| anyhow!("the sound output takes no 48 kHz float samples"))?;
        let channels = usize::from(config.channels()).max(1);
        let mut one_channel = Vec::new();
        let stream = device.build_output_stream(
            config.config(),
            move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let times = info.timestamp();
                // How long after this call its first sample leaves the machine.
                let ahead: Duration = times.playback.duration_since(times.callback);
                one_channel.clear();
                one_channel.resize(out.len() / channels, 0.0);
                fill(Instant::now() + ahead, &mut one_channel);
                for (frame, value) in out.chunks_mut(channels).zip(&one_channel) {
                    frame.fill(*value);
                }
            },
            |error| eprintln!("the sound output: {error}"),
            None,
        )?;
        stream.play()?;
        Ok(Self { _stream: stream })
    }
}
