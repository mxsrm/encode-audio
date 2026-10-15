//! Input demuxing and audio-relative track selection.

use std::path::Path;

use anyhow::{Context, Result};
use ffmpeg_next::{Dictionary, codec, format, media};

/// Owns the demuxer and the decoder for the selected track.
pub(crate) struct Input {
    pub context: format::context::Input,
    pub decoder: codec::decoder::Audio,
    pub stream_index: usize,
    pub metadata: Dictionary<'static>,
    pub precision: i32,
}

impl Input {
    pub fn open(path: &Path, audio_index: usize) -> Result<Self> {
        let context = format::input(path).context("opening input media")?;
        let stream = context
            .streams()
            .filter(|s| s.parameters().medium() == media::Type::Audio)
            .nth(audio_index)
            .with_context(|| format!("audio track a:{audio_index} does not exist"))?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let metadata = stream.metadata().to_owned();
        let mut decoder = codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .audio()
            .context("opening audio decoder")?;
        decoder.set_packet_time_base(time_base);
        // SAFETY: decoder owns a live AVCodecContext. The wrapper has no
        // accessor for this read-only sample-precision field.
        let precision = unsafe { (*decoder.as_ptr()).bits_per_raw_sample };
        Ok(Self {
            context,
            decoder,
            stream_index,
            metadata,
            precision,
        })
    }
}
