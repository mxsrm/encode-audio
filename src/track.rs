//! One output track: encoder configuration, conversion, and packet muxing.

use anyhow::{
    Context,
    Result,
    bail,
    ensure,
};
use ffmpeg::{
    ChannelLayout,
    Dictionary,
    Packet,
    codec,
    format,
    frame,
};
use ffmpeg_next as ffmpeg;

use crate::{
    config::{
        Codec,
        Config,
        LayoutPolicy,
        output_layout,
    },
    filters::AudioFilter,
};

pub(crate) struct Track {
    encoder: codec::encoder::audio::Encoder,
    filter: AudioFilter,
    stream: usize,
    codec: Codec,
    samples: u64,
    payload_packets: u64,
}

impl Track {
    /// Add a fully configured output stream before the muxer writes its header.
    pub fn new(
        kind: Codec,
        config: &Config,
        source: &frame::Audio,
        metadata: &Dictionary<'_>,
        output: &mut format::context::Output,
    ) -> Result<Self> {
        let name = match kind {
            Codec::Opus => "libopus",
            Codec::Flac => "flac",
            Codec::Truehd => "truehd",
            Codec::Dual => bail!("dual is a container mode, not an encoder"),
        };
        let codec = ffmpeg::encoder::find_by_name(name)
            .with_context(|| format!("encoder {name} unavailable in linked FFmpeg build"))?;
        let audio_codec = codec.audio()?;
        let layout = if kind == Codec::Truehd {
            truehd_layout(source.channel_layout(), config.layout, &audio_codec)?
        } else {
            output_layout(source.channel_layout(), config.layout)?
        };
        if kind == Codec::Opus {
            ensure!(
                (1..=8).contains(&layout.channels()),
                "Opus supports at most 8 channels in mapping family 1"
            );
        }
        if let Some(layouts) = audio_codec.channel_layouts() {
            ensure!(
                layouts.into_iter().any(|l| l == layout),
                "{name} does not support requested channel layout {layout:?}"
            );
        }
        let rate = if kind == Codec::Opus { 48_000 } else { source.rate() };
        ensure!(rate > 0 && rate <= i32::MAX as u32, "invalid source sample rate");
        if let Some(rates) = audio_codec.rates() {
            ensure!(
                rates.into_iter().any(|r| r == rate as i32),
                "{name} does not support {rate} Hz; refusing to resample lossless output"
            );
        }
        let formats: Vec<_> = audio_codec
            .formats()
            .context("encoder advertises no sample formats")?
            .collect();
        let preferred = match kind {
            Codec::Opus => format::Sample::F32(format::sample::Type::Packed),
            Codec::Truehd if source.format().bytes() <= 2 => format::Sample::I16(format::sample::Type::Planar),
            Codec::Truehd => format::Sample::I32(format::sample::Type::Planar),
            _ if source.format().bytes() <= 2 => format::Sample::I16(format::sample::Type::Packed),
            _ => format::Sample::I32(format::sample::Type::Packed),
        };
        ensure!(
            formats.contains(&preferred),
            "{name} lacks required sample format {preferred:?}"
        );
        let mut encoder = codec::context::Context::new_with_codec(codec).encoder().audio()?;
        encoder.set_rate(rate as i32);
        encoder.set_channel_layout(layout);
        #[cfg(legacy_channel_count)]
        encoder.set_channels(layout.channels());
        encoder.set_format(preferred);
        encoder.set_time_base((1, rate as i32));
        if output.format().flags().contains(format::flag::Flags::GLOBAL_HEADER) {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }
        let mut options = Dictionary::new();
        let bitrate = config.bitrate_per_channel as usize * source.channels() as usize * 1000;
        match kind {
            Codec::Opus => {
                ensure!(
                    (6_000..=512_000).contains(&bitrate),
                    "total Opus bitrate {bitrate} is outside libopus's 6000..=512000 bit/s range"
                );
                encoder.set_bit_rate(bitrate);
                options.set("vbr", "on");
                options.set("mapping_family", if layout.channels() > 2 { "1" } else { "0" });
            }
            Codec::Flac => options.set("compression_level", "8"),
            // FFmpeg marks this encoder experimental. This opt-in is confined
            // to TrueHD rather than weakening compliance for all encoders.
            Codec::Truehd => {
                options.set("strict", "experimental");
                options.set("max_interval", "16");
            }
            Codec::Dual => unreachable!(),
        }
        let encoder = encoder
            .open_as_with(codec, options)
            .with_context(|| format!("opening {name} encoder"))?;
        let mut stream = output.add_stream(codec)?;
        let stream_index = stream.index();
        stream.set_parameters(&encoder);
        stream.set_time_base(encoder.time_base());
        let mut tags = metadata.to_owned();
        if config.codec == Codec::Dual || kind == Codec::Truehd {
            let title = match kind {
                Codec::Opus => format!("Opus ({}k)", bitrate / 1000),
                Codec::Flac => "FLAC (Lossless)".to_owned(),
                _ => "TrueHD (Lossless)".to_owned(),
            };
            tags.set("title", &title);
        }
        stream.set_metadata(tags);
        let filter = AudioFilter::new(source, &encoder)?;
        Ok(Self {
            encoder,
            filter,
            stream: stream_index,
            codec: kind,
            samples: 0,
            payload_packets: 0,
        })
    }

    pub fn push(&mut self, frame: &frame::Audio, output: &mut format::context::Output) -> Result<()> {
        self.filter.push(frame)?;
        self.drain_filter(output, false)
    }

    /// Flush resampler/FIFO before sending encoder EOF so delayed tail samples
    /// are included. After EOF, EAGAIN is treated as an unexpected state.
    pub fn finish(&mut self, output: &mut format::context::Output) -> Result<()> {
        self.filter.finish()?;
        self.drain_filter(output, true)?;
        if self.codec == Codec::Truehd {
            // The encoder buffers a major-sync interval. Very short inputs
            // can otherwise yield packets that decode to no samples on 6.x.
            let minimum = self.encoder.frame_size() as u64 * 16;
            ensure!(
                self.samples >= minimum,
                "TrueHD input is too short: need at least {minimum} samples per channel for a complete major-sync \
                 interval"
            );
        }
        self.encoder.send_eof().context("flushing encoder")?;
        self.drain_packets(output, true)?;
        ensure!(self.payload_packets > 0, "encoder produced no audio packets");
        Ok(())
    }

    fn drain_filter(&mut self, output: &mut format::context::Output, finishing: bool) -> Result<()> {
        loop {
            let mut frame = frame::Audio::empty();
            match self.filter.receive(&mut frame) {
                Ok(()) => {
                    // Audio extraction intentionally uses a continuous sample
                    // timeline. Count encoder-rate samples rather than round
                    // sink PTS from a possibly coarse time base. TrueHD's
                    // small frames otherwise overlap or move backward.
                    frame.set_pts(Some(
                        i64::try_from(self.samples).context("encoder sample timestamp overflow")?,
                    ));
                    self.encoder.send_frame(&frame).context("sending frame to encoder")?;
                    self.samples = self
                        .samples
                        .checked_add(frame.samples() as u64)
                        .context("encoded sample counter overflow")?;
                    self.drain_packets(output, false)?;
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(e) if is_again(e) && !finishing => return Ok(()),
                Err(e) => return Err(e).context("receiving filtered audio"),
            }
        }
    }

    fn drain_packets(&mut self, output: &mut format::context::Output, finishing: bool) -> Result<()> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    if packet.size() > 0 {
                        self.payload_packets += 1;
                    }
                    // Muxers may rewrite stream time bases when writing headers.
                    let time_base = output.stream(self.stream).context("missing output stream")?.time_base();
                    packet.rescale_ts(self.encoder.time_base(), time_base);
                    packet.set_stream(self.stream);
                    packet.set_position(-1);
                    crate::mux::write(&mut packet, output)
                        .with_context(|| format!("muxing {:?} packet", self.codec))?;
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(e) if is_again(e) && !finishing => return Ok(()),
                Err(e) => return Err(e).context("receiving encoded packet"),
            }
        }
    }
}

/// Use the linked encoder's advertised layouts. TrueHD commonly wants side
/// channels while libopus wants back channels; never silently reduce channels.
fn truehd_layout(source: ChannelLayout, policy: LayoutPolicy, codec: &codec::Audio) -> Result<ChannelLayout> {
    let supported: Vec<_> = codec
        .channel_layouts()
        .context("TrueHD advertises no layouts")?
        .collect();
    if supported.contains(&source) {
        return Ok(source);
    }
    if policy == LayoutPolicy::Compatible {
        // Newer FFmpeg bindings store AVChannelLayout's union internally.
        // Such constants cannot be used in Rust structural match patterns;
        // the wrapper's PartialEq implementation compares them safely.
        let equivalent = if source == ChannelLayout::_5POINT1_BACK {
            Some(ChannelLayout::_5POINT1)
        } else if source == ChannelLayout::_5POINT0_BACK {
            Some(ChannelLayout::_5POINT0)
        } else {
            None
        };
        if let Some(layout) = equivalent.filter(|l| supported.contains(l)) {
            return Ok(layout);
        }
    }
    bail!("linked TrueHD encoder does not support source layout {source:?}; supported: {supported:?}")
}

pub(crate) fn is_again(error: ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if errno == ffmpeg::error::EAGAIN)
}

#[cfg(test)]
mod tests {
    use ffmpeg::Rescale;

    use super::*;

    #[test]
    fn truehd_planar_pcm_from_millisecond_timestamps_muxes_without_dts_regression() {
        ffmpeg::init().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("planar.mka");
        let mut output = format::output_as(&path, "matroska").unwrap();
        let config = Config {
            input: path.clone(),
            output: None,
            codec: Codec::Truehd,
            audio_stream: 0,
            bitrate_per_channel: 64,
            layout: LayoutPolicy::Preserve,
        };
        let make_frame = || {
            let mut source = frame::Audio::new(
                format::Sample::I32(format::sample::Type::Planar),
                1024,
                ChannelLayout::STEREO,
            );
            source.set_rate(48000);
            source
        };
        let source = make_frame();
        let mut track = Track::new(Codec::Truehd, &config, &source, &Dictionary::new(), &mut output).unwrap();
        output.write_header().unwrap();
        for offset in (0..48 * 1024).step_by(1024) {
            let mut source = make_frame();
            for channel in 0..2 {
                for (i, sample) in source.plane_mut::<i32>(channel).iter_mut().enumerate() {
                    *sample = (((offset + i) as i32 % 16001) - 8000) << 8;
                }
            }
            source.set_pts(Some((offset as i64).rescale((1, 48000), (1, 1000))));
            track.push(&source, &mut output).unwrap();
        }
        track.finish(&mut output).unwrap();
        output.write_trailer().unwrap();
    }
}
