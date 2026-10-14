# Validation record

Validated on Linux with Rust 1.90.0, ffmpeg-next 8.1.0, and FFmpeg 6.1.1.

- Rust compilation and executable build passed.
- Six Rust integration tests passed, including exact lossless PCM checks.
- Twenty-five independent FFmpeg/ffprobe media checks passed.
- Rustfmt checks passed.
- Clippy checks for all targets passed with warnings denied.
- Rustdoc generation passed with warnings denied.

Media coverage includes mono, stereo, 5.1, 7.1, 16/24-bit PCM, 44.1/48 kHz,
FLAC, Opus, dual output, and TrueHD. Tests check TrueHD partial tails,
unsupported rates/layouts and very short inputs, temporary-file cleanup,
existing-output protection, and audio-relative track selection in a movie.

Production code contains no subprocess encoding or probing calls. Only the
optional Python integration oracle invokes ffmpeg/ffprobe.

The sink compatibility fix was additionally compiled and exercised on Linux
against a locally built FFmpeg 8.0 with ffmpeg-next 9.0.0: all six Rust
integration tests and all twenty-five independent media checks passed.
The default package continues to pin ffmpeg-next 8.1.0. macOS was not tested.
All six Rust integration tests also passed with the pinned 8.1.0 bindings
against FFmpeg 8.0.
The supplied CI workflow was included for future runs; no remote CI was run.

Generated HTML API documentation is included at rustdoc/encode_audio/index.html.
