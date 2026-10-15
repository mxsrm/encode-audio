//! Thin executable entry point for the reusable encoding library.

use std::process::ExitCode;

use clap::Parser;
use encode_audio::{Config, cli::Cli, encode};

fn main() -> ExitCode {
    let config = Config::from(Cli::parse());
    match encode(&config) {
        Ok(report) => {
            println!(
                "Encoded audio track {} ({} channels, {} decoded samples) to {}",
                config.audio_stream,
                report.channels,
                report.decoded_samples,
                report.output.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
