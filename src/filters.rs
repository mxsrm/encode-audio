//! Per-encoder conversion of sample formats, rates, layouts, and frame sizes.

use anyhow::{Context, Result, ensure};
use ffmpeg_next::{ChannelLayout, codec, filter, frame};

pub(crate) struct AudioFilter {
    graph: filter::Graph,
    rate: u32,
    format: ffmpeg_next::format::Sample,
    layout: ChannelLayout,
}

impl AudioFilter {
    /// Negotiate conversion from actual decoded properties, not guessed metadata.
    pub fn new(source: &frame::Audio, encoder: &codec::encoder::audio::Encoder) -> Result<Self> {
        let mut graph = filter::Graph::new();
        let rate = source.rate();
        let args = format!(
            "time_base=1/{rate}:sample_rate={rate}:sample_fmt={}:channel_layout=0x{:x}",
            source.format().name(),
            source.channel_layout().bits()
        );
        graph.add(
            &filter::find("abuffer").context("abuffer filter unavailable")?,
            "source",
            &args,
        )?;
        graph.add(
            &filter::find("abuffersink").context("abuffersink filter unavailable")?,
            "sink",
            "",
        )?;
        // Express conversion constraints through aformat's text options. The
        // sink's legacy channel_layouts AVOption was removed in newer FFmpeg,
        // and the wrapper's typed setters ignore option-setting errors.
        // Parsing/validation here propagates failures instead of silently
        // leaving an unconstrained sink.
        let spec = format!(
            "[source]aformat=sample_fmts={}:sample_rates={}:channel_layouts=0x{:x},asetpts=N/SR/TB[sink]",
            encoder.format().name(),
            encoder.rate(),
            encoder.channel_layout().bits()
        );
        // The aformat constraints trigger automatic resampling/remixing. A sample
        // timeline starts at zero and retains every decoded sample, even when
        // source timestamps are absent or start at a movie-specific offset.
        graph.output("source", 0)?.input("sink", 0)?.parse(&spec)?;
        graph.validate().context("negotiating audio conversion")?;
        if encoder.frame_size() > 0 {
            graph
                .get("sink")
                .context("missing filter sink")?
                .sink()
                .set_frame_size(encoder.frame_size());
        }
        Ok(Self {
            graph,
            rate: source.rate(),
            format: source.format(),
            layout: source.channel_layout(),
        })
    }

    /// FFmpeg's source API consumes frame references; use an independent copy
    /// for each branch so dual mode never feeds an already-consumed frame.
    pub fn push(&mut self, source: &frame::Audio) -> Result<()> {
        ensure!(
            source.rate() == self.rate && source.format() == self.format && source.channel_layout() == self.layout,
            "input audio properties changed midstream; conversion cannot continue safely"
        );
        // SAFETY: source owns a live AVFrame. av_frame_clone allocates an
        // independent frame header and retains the underlying audio buffers.
        // Audio::wrap takes ownership of that header and frees it on drop.
        // Using FFmpeg's reference clone also preserves both legacy and modern
        // layout fields, which the wrapper's deep clone can fail to copy on 6.x.
        let copy = unsafe {
            let pointer = ffmpeg_next::ffi::av_frame_clone(source.as_ptr());
            ensure!(!pointer.is_null(), "allocating filter input frame failed");
            frame::Audio::wrap(pointer)
        };
        self.graph
            .get("source")
            .context("missing filter source")?
            .source()
            .add(&copy)?;
        Ok(())
    }

    pub fn receive(&mut self, frame: &mut frame::Audio) -> Result<(), ffmpeg_next::Error> {
        self.graph
            .get("sink")
            .ok_or(ffmpeg_next::Error::FilterNotFound)?
            .sink()
            .frame(frame)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.graph
            .get("source")
            .context("missing filter source")?
            .source()
            .flush()?;
        Ok(())
    }
}
