//! Command-line parsing. Encoding logic remains usable independently of clap.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::config::{Codec, Config, LayoutPolicy};

/// Extract one audio track and encode it using FFmpeg libraries directly.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Input media file (audio or video).
    #[arg(short, long)]
    pub input: PathBuf,
    /// Output file; defaults to `<input_stem>_track<N>.<opus|flac|mka>`.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Target codec; dual writes Opus and FLAC tracks into Matroska.
    #[arg(short, long, value_enum, ignore_case = true, default_value = "opus")]
    pub codec: Codec,
    /// Zero-based AUDIO track index, not the absolute media stream index.
    #[arg(short = 's', long, default_value_t = 0)]
    pub stream: usize,
    /// Compatibility option: only the native library backend is supported.
    #[arg(
        short = 'e',
        long = "encoder",
        value_enum,
        ignore_case = true,
        default_value = "native"
    )]
    pub encoder: Backend,
    /// Opus kilobits/second per source channel (FLAC ignores this).
    #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u32).range(1..=512))]
    pub bitrate_per_channel: u32,
    /// Standardize mono/stereo/5.1/7.1 layouts, or preserve the source layout.
    #[arg(long, value_enum, default_value = "compatible")]
    pub layout: LayoutPolicy,
}

/// The sole supported backend; no child encoding processes are launched.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Backend {
    /// FFmpeg's library API, including its libopus encoder.
    Native,
}

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Self {
            input: cli.input,
            output: cli.output,
            codec: cli.codec,
            audio_stream: cli.stream,
            bitrate_per_channel: cli.bitrate_per_channel,
            layout: cli.layout,
        }
    }
}
