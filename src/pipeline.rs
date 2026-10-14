//! Decode once, feed independent output branches, and drain every stage.

use std::path::PathBuf;

use anyhow::{
    Context,
    Result,
    ensure,
};
use ffmpeg_next::{
    ChannelLayout,
    Packet,
    format,
    frame,
};

use crate::{
    Codec,
    Config,
    input::Input,
    output::OutputFile,
    track::{
        Track,
        is_again,
    },
};

/// Summary returned only after the completed output has been published.
#[derive(Debug, Clone)]
pub struct EncodeReport {
    /// Final output path.
    pub output: PathBuf,
    /// Number of source channels.
    pub channels: u16,
    /// Decoded samples per channel, before any Opus resampling.
    pub decoded_samples: u64,
    /// Number of encoded output tracks.
    pub tracks: usize,
}

/// Extract and encode the configured audio track.
///
/// This synchronous function initializes FFmpeg and uses its API directly.
/// Existing output paths (including dangling symlinks) are never overwritten.
/// Ordinary errors remove the temporary output. A failure during final
/// publication also leaves any pre-existing destination intact.
///
/// # Errors
/// Returns contextual errors for missing tracks/codecs, unsupported layouts or
/// sample rates, corrupt input, conversion/encoding failures, and filesystem
/// errors. Midstream changes to audio properties are rejected.
pub fn encode(config: &Config) -> Result<EncodeReport> {
    let destination = config.validate()?;
    ffmpeg_next::init().context("initializing FFmpeg")?;
    let mut input = Input::open(&config.input, config.audio_stream)?;
    ensure!(
        config.codec == Codec::Opus || input.precision <= 24,
        "lossless modes support up to 24-bit source precision; input reports {} bits",
        input.precision
    );
    let file = OutputFile::new(&destination)?;
    let mut output = format::output_as(file.path(), config.codec.muxer()).context("opening muxer")?;
    output.set_metadata(input.context.metadata().to_owned());
    let mut pipeline = Pipeline {
        tracks: Vec::new(),
        channels: 0,
        samples: 0,
    };
    loop {
        let mut packet = Packet::empty();
        // Unlike the convenience packets() iterator, this preserves demuxer
        // read errors rather than treating every error as successful EOF.
        match packet.read(&mut input.context) {
            Ok(()) if packet.stream() == input.stream_index => {
                input
                    .decoder
                    .send_packet(&packet)
                    .context("sending packet to decoder")?;
                pipeline.drain(&mut input, config, &mut output, false)?;
            }
            Ok(()) => {}
            Err(ffmpeg_next::Error::Eof) => break,
            Err(e) => return Err(e).context("reading input packet"),
        }
    }
    input.decoder.send_eof().context("flushing decoder")?;
    pipeline.drain(&mut input, config, &mut output, true)?;
    ensure!(!pipeline.tracks.is_empty(), "selected audio track decoded no samples");
    for track in &mut pipeline.tracks {
        track.finish(&mut output)?;
    }
    output.write_trailer().context("finalizing output container")?;
    drop(output);
    file.commit()?;
    Ok(EncodeReport {
        output: destination,
        channels: pipeline.channels,
        decoded_samples: pipeline.samples,
        tracks: pipeline.tracks.len(),
    })
}

struct Pipeline {
    tracks: Vec<Track>,
    channels: u16,
    samples: u64,
}

impl Pipeline {
    fn drain(
        &mut self,
        input: &mut Input,
        config: &Config,
        output: &mut format::context::Output,
        finishing: bool,
    ) -> Result<()> {
        loop {
            let mut decoded = frame::Audio::empty();
            match input.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    ensure!(
                        decoded.rate() > 0 && decoded.channels() > 0,
                        "decoded audio has invalid properties"
                    );
                    if decoded.channel_layout().is_empty() {
                        // FFmpeg's conventional layout is the only assumption
                        // made when source metadata contains just a count.
                        decoded.set_channel_layout(ChannelLayout::default(decoded.channels() as i32));
                    }
                    ensure!(!decoded.channel_layout().is_empty(), "cannot infer source layout");
                    // The filter source uses 1/sample_rate, independent of the
                    // demuxer's often coarse (e.g. millisecond) time base.
                    decoded.set_pts(Some(
                        i64::try_from(self.samples).context("decoded sample timestamp overflow")?,
                    ));
                    if self.tracks.is_empty() {
                        self.channels = decoded.channels();
                        let kinds: &[Codec] = match config.codec {
                            Codec::Dual => &[Codec::Opus, Codec::Flac],
                            Codec::Opus => &[Codec::Opus],
                            Codec::Flac => &[Codec::Flac],
                            Codec::Truehd => &[Codec::Truehd],
                        };
                        for &kind in kinds {
                            self.tracks
                                .push(Track::new(kind, config, &decoded, &input.metadata, output)?);
                        }
                        output.write_header().context("writing container header")?;
                    }
                    self.samples = self
                        .samples
                        .checked_add(decoded.samples() as u64)
                        .context("sample counter overflow")?;
                    for track in &mut self.tracks {
                        track.push(&decoded, output)?;
                    }
                }
                Err(ffmpeg_next::Error::Eof) => return Ok(()),
                Err(e) if is_again(e) && !finishing => return Ok(()),
                Err(e) => return Err(e).context("decoding audio frame"),
            }
        }
    }
}
