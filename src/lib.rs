//! Audio extraction and transcoding through FFmpeg libraries.
//!
//! [`encode`] selects an audio-relative stream, decodes it once, and sends each
//! decoded frame through independent filter/encoder branches. Dual mode muxes
//! Opus followed by FLAC into Matroska without intermediate encoded files.
//! Outputs are published without overwriting existing files only after the
//! decoder, filters, encoders, and muxer have all finished successfully.

pub mod cli;
pub mod config;
mod filters;
mod input;
mod mux;
mod output;
mod pipeline;
mod track;

pub use config::{
    Codec,
    Config,
    LayoutPolicy,
};
pub use pipeline::{
    EncodeReport,
    encode,
};
