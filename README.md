# encode-audio

A standalone Rust replacement for the **native FFmpeg encoding path** in
`encode-audio.sh`. Uses clap for argument parsing and ffmpeg-next 8.1.0 for
demuxing, decoding, audio conversion, encoding, and muxing. The application
never launches `ffmpeg`, `ffprobe`, `flac`, or `opusenc` processes.

## Build

Install Rust 1.85 or newer, a C toolchain, pkg-config, libclang, and FFmpeg
development libraries. FFmpeg must include the `libopus`, `flac`, and `truehd`
encoders and the Ogg, FLAC, and Matroska muxers for the respective modes.
The executable links to FFmpeg libraries; it does not bundle them.

Ubuntu 24.04 / Debian:

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config clang libclang-dev \
  libavcodec-dev libavformat-dev libavfilter-dev libavutil-dev \
  libswresample-dev libswscale-dev libpostproc-dev
cargo build --release --locked
```

macOS with Homebrew:

```sh
brew install ffmpeg pkg-config llvm
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
cargo build --release --locked
```

Validated on Linux with Rust 1.90 and FFmpeg 6.1.1, and against a locally built
FFmpeg 8.0 with ffmpeg-next 9.0.0. The default dependency remains pinned to
ffmpeg-next 8.1.0. The build script selects
the channel-count compatibility API from the installed libavcodec version;
the code also uses ffmpeg-next's automatically detected FFmpeg 7/8 APIs.
macOS itself has not been exercised here.

## Usage

```sh
encode-audio -i movie.mkv                         # Opus, first audio track
encode-audio -i movie.mkv -c dual -e native       # Opus + FLAC in .mka
encode-audio -i movie.mp4 -c flac -s 1 -o music.flac
encode-audio -i movie.mkv -c truehd               # TrueHD in .mka
encode-audio -i source.wav -c flac --layout preserve
encode-audio -i movie.mkv -c opus --bitrate-per-channel 48
encode-audio --help
```

Run `target/release/encode-audio` directly, or install using `cargo install
--path . --locked`. `-s 1` selects the **second audio track**, even if video
or subtitle streams precede it. Codec values accept upper or lower case.
`-e native` is accepted for script compatibility; `external` is rejected.

| Mode | Encoder | Container | Default filename |
| --- | --- | --- | --- |
| `opus` | libopus, VBR, 64 kb/s per source channel | Ogg | `<stem>_track<N>.opus` |
| `flac` | FLAC, compression level 8 | FLAC | `<stem>_track<N>.flac` |
| `dual` | Opus then FLAC | Matroska | `<stem>_track<N>.mka` |
| `truehd` | FFmpeg TrueHD, experimental compliance | Matroska | `<stem>_track<N>.mka` |

The codec explicitly selects the container, even when `-o` supplies a different
extension. Use the extensions above to avoid misleading filenames. The program
accepts regular local input files; URLs, stdin, and output pipes are not supported.

Opus uses 48 kHz, mapping family 0 for mono/stereo and family 1 for surround.
The total bitrate is computed from the original channel count, as in the script;
libopus's 6–512 kb/s total limit is validated. FLAC and TrueHD retain the source
sample rate. Unsupported lossless sample rates cause an error instead of resampling.

Dual-track titles are `Opus (<bitrate>k)` and `FLAC (Lossless)`. TrueHD is titled
`TrueHD (Lossless)`. Input container metadata and selected-track metadata such
as language are copied when the output format supports them.

## Layout and lossless behavior

The default `--layout compatible` standardizes mono, stereo, six-channel audio
to 5.1(back), and eight-channel audio to 7.1 for Opus/FLAC. In particular,
5.1(side) is converted to the layout accepted by libopus. Unlike the script's
unconstrained `aformat` negotiation, other channel counts produce an error
instead of silently changing their count. Use `--layout preserve` for those
inputs or for exact lossless PCM preservation. Conversion between side and
back layouts is performed by FFmpeg's resampler, not by relabeling samples.

TrueHD independently checks the linked encoder's advertised layouts. Compatible
mode preserves supported layouts and converts 5.0/5.1(back) to their side-channel
equivalent when needed. Preserve mode requires the exact layout to be supported.
With FFmpeg 6.1.1, TrueHD accepts mono, stereo, 2.1, 3.0, 3.1, 4.0, 4.1,
5.0(side), and 5.1(side); **7.1 is rejected without downmixing**. Its advertised
rates are 44100, 48000, 88200, 96000, 176400, and 192000 Hz.

TrueHD explicitly enables experimental compliance only for that encoder and
uses a 16-frame major-sync interval. Inputs shorter than one interval are
rejected because FFmpeg 6 can otherwise produce undecodable short outputs
(640 samples per channel at 48 kHz, about 13.3 ms). TrueHD encoding creates
channel-based audio; Dolby Atmos object metadata is not recreated or preserved.

"Lossless" describes the codec. Layout conversion or quantization of floating
point PCM changes source samples. Known integer source precision above 24 bits
is rejected in lossless modes. Where the input omits precision, FFmpeg's
16/24-bit conversion is used. Exact preservation is verified for 16- and
24-bit integer PCM with an unchanged layout and sample rate. Re-encoding a
lossy source cannot recover information previously discarded.

## Structure

| Module | Responsibility |
| --- | --- |
| `main` / `cli` | clap options, status output, exit code |
| `config` | output naming, validation, channel-layout policy |
| `input` | audio-relative selection and decoder initialization |
| `pipeline` | demux/decode loop and one-time initialization from actual PCM |
| `track` | encoder options, independent output branches, draining |
| `filters` | sample format/rate/layout conversion and frame-size buffering |
| `mux` | metadata-only packet compatibility for FLAC final STREAMINFO |
| `output` | temporary output, cleanup, no-overwrite publication |

Dual mode decodes once. Each branch gets a reference-counted frame header,
its own filter graph, and its own encoder. PCM buffers remain shared until
FFmpeg needs to modify them. No temporary Opus or FLAC files are needed.

Output sample format, rate, and layout are negotiated through an explicit
`aformat` filter. No deprecated typed sink-option setters are used; graph
parsing and validation return errors if the requested conversion is unsupported.

The output header is written after the first decoded frame establishes real
audio properties. Encoded timestamps are rescaled from the encoder time base
to the muxer's **post-header** stream time base. Extracted audio starts at zero
and keeps a continuous sample timeline; source timestamp gaps and video sync
offsets are not retained. Midstream audio-property changes are rejected.

At EOF the program drains decoder → filters/resamplers → encoders → muxer
trailer, distinguishing EAGAIN/EOF from actual errors. The completed temporary
file is synced and published without overwriting a destination that appeared
mid-encode. Failures and panic unwinding clean up temporary output; a hard kill
can leave `.encode-audio-*` files in the destination directory. Source files
and existing outputs remain untouched. The temporary directory is the output
directory so publication stays on the same filesystem.

Public APIs and implementation modules have rustdoc comments. Library callers
can construct `encode_audio::Config` and call `encode_audio::encode(&config)`;
FFmpeg errors are returned with context through `anyhow::Result`.

## Validation and documentation

```sh
cargo fmt --check
cargo test --locked
cargo doc --no-deps --locked
# Optional independent oracle, requiring ffmpeg/ffprobe only for testing:
cargo build --locked
python3 tests/verify_media.py target/debug/encode-audio
```

The Rust integration tests generate WAV fixtures and decode their output with
the bindings. They verify exact FLAC PCM, TrueHD PCM including partial tails,
dual stream order/titles, resampling, and failure cleanup. The independent
Python harness uses FFmpeg/ffprobe to verify mono, stereo, 5.1, 7.1, 16/24-bit PCM,
codec/container properties, sample counts, second-track extraction from video,
5.1(side) conversion, and rejection of unsupported TrueHD 7.1.

API references: [ffmpeg-next](https://docs.rs/ffmpeg-next/8.1.0/ffmpeg_next/),
[clap](https://docs.rs/clap/latest/clap/),
[FFmpeg encoding and filters](https://ffmpeg.org/documentation.html).

