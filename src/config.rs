//! Validated encoding choices and deterministic output naming.

use std::path::{
    Path,
    PathBuf,
};

use anyhow::{
    Context,
    Result,
    ensure,
};
use clap::ValueEnum;
use ffmpeg_next::ChannelLayout;

/// Requested output codec or pair of codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Codec {
    /// Variable-bitrate libopus audio in Ogg.
    Opus,
    /// FLAC audio with compression level 8.
    Flac,
    /// FFmpeg TrueHD lossless audio in Matroska (experimental encoder).
    Truehd,
    /// Opus and FLAC audio tracks in a Matroska container.
    Dual,
}

impl Codec {
    /// Default output suffix.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Flac => "flac",
            Self::Dual | Self::Truehd => "mka",
        }
    }

    pub(crate) fn muxer(self) -> &'static str {
        match self {
            Self::Opus => "ogg",
            Self::Flac => "flac",
            Self::Dual | Self::Truehd => "matroska",
        }
    }
}

/// Policy for channel-layout conversion before encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LayoutPolicy {
    /// Match the script's common layouts; convert 5.1(side) to 5.1(back).
    /// Reject other channel counts rather than silently changing their count.
    Compatible,
    /// Preserve the source layout. The selected encoder must support it.
    Preserve,
}

/// Library configuration. Paths refer to local files, not URLs or pipes.
#[derive(Debug, Clone)]
pub struct Config {
    /// Input media file.
    pub input: PathBuf,
    /// Explicit output path, or `None` for automatic naming.
    pub output: Option<PathBuf>,
    /// Output codec(s).
    pub codec: Codec,
    /// Zero-based index among audio streams only.
    pub audio_stream: usize,
    /// Opus kilobits/second per source channel.
    pub bitrate_per_channel: u32,
    /// Channel-layout conversion policy.
    pub layout: LayoutPolicy,
}

impl Config {
    /// Resolve the output name without accessing the filesystem.
    pub fn output_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.output {
            return Ok(path.clone());
        }
        let stem = self.input.file_stem().context("input has no filename stem")?;
        let mut name = stem.to_os_string();
        name.push(format!("_track{}.{}", self.audio_stream, self.codec.extension()));
        Ok(self.input.with_file_name(name))
    }

    pub(crate) fn validate(&self) -> Result<PathBuf> {
        ensure!(
            self.input.is_file(),
            "input is not a regular file: {}",
            self.input.display()
        );
        ensure!(
            (1..=512).contains(&self.bitrate_per_channel),
            "bitrate must be 1..=512 kb/s per channel"
        );
        let output = self.output_path()?;
        ensure!(output.file_name().is_some(), "output must have a filename");
        // symlink_metadata also detects dangling symlinks.
        match std::fs::symlink_metadata(&output) {
            Ok(_) => anyhow::bail!("output already exists: {}", output.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("checking output path"),
        }
        ensure!(parent(&output).is_dir(), "output directory does not exist");
        Ok(output)
    }
}

pub(crate) fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub(crate) fn output_layout(source: ChannelLayout, policy: LayoutPolicy) -> Result<ChannelLayout> {
    ensure!(!source.is_empty(), "source channel layout is unknown");
    if policy == LayoutPolicy::Preserve {
        return Ok(source);
    }
    Ok(match source.channels() {
        1 => ChannelLayout::MONO,
        2 => ChannelLayout::STEREO,
        6 => ChannelLayout::_5POINT1_BACK,
        8 => ChannelLayout::_7POINT1,
        n => anyhow::bail!("{n}-channel audio needs --layout preserve; compatible supports 1, 2, 6, or 8 channels"),
    })
}
