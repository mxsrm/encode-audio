//! End-to-end tests using generated PCM WAV input and FFmpeg decoding as an
//! independent check of the produced container and audio samples.

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
};

use encode_audio::{
    Codec,
    Config,
    LayoutPolicy,
    encode,
};
use ffmpeg_next::{
    Packet,
    codec,
    format,
    frame,
    media,
};

fn wav(path: &Path, channels: u16, samples: usize, rate: u32) -> Vec<i16> {
    let pcm: Vec<i16> = (0..samples)
        .flat_map(|i| (0..channels).map(move |c| ((i as i32 * (37 + c as i32 * 101)) % 16001 - 8000) as i16))
        .collect();
    let bytes = (pcm.len() * 2) as u32;
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + bytes).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    file.extend_from_slice(&16u32.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&channels.to_le_bytes());
    file.extend_from_slice(&rate.to_le_bytes());
    file.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    file.extend_from_slice(&(channels * 2).to_le_bytes());
    file.extend_from_slice(&16u16.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&bytes.to_le_bytes());
    for sample in &pcm {
        file.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, file).unwrap();
    pcm
}

fn config(input: &Path, output: PathBuf, codec: Codec) -> Config {
    Config {
        input: input.to_owned(),
        output: Some(output),
        codec,
        audio_stream: 0,
        bitrate_per_channel: 64,
        layout: LayoutPolicy::Preserve,
    }
}

fn decode_i16(path: &Path, audio_index: usize) -> Vec<i16> {
    let mut input = format::input(path).unwrap();
    let stream = input
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Audio)
        .nth(audio_index)
        .unwrap();
    let index = stream.index();
    let mut decoder = codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    let mut pcm = Vec::new();
    let drain = |decoder: &mut codec::decoder::Audio, pcm: &mut Vec<i16>| loop {
        let mut audio = frame::Audio::empty();
        match decoder.receive_frame(&mut audio) {
            Ok(()) => {
                assert!(matches!(
                    audio.format(),
                    format::Sample::I16(_) | format::Sample::I32(_)
                ));
                for i in 0..audio.samples() {
                    for c in 0..audio.channels() as usize {
                        let sample = if matches!(audio.format(), format::Sample::I32(_)) {
                            // TrueHD's decoder expands integer PCM to s32.
                            if audio.is_planar() {
                                (audio.plane::<i32>(c)[i] >> 16) as i16
                            } else {
                                let offset = (i * audio.channels() as usize + c) * 4;
                                let bytes: [u8; 4] = audio.data(0)[offset..offset + 4].try_into().unwrap();
                                (i32::from_ne_bytes(bytes) >> 16) as i16
                            }
                        } else if audio.is_planar() {
                            audio.plane::<i16>(c)[i]
                        } else {
                            let offset = (i * audio.channels() as usize + c) * 2;
                            let data = audio.data(0);
                            i16::from_ne_bytes([data[offset], data[offset + 1]])
                        };
                        pcm.push(sample);
                    }
                }
            }
            Err(ffmpeg_next::Error::Eof) => break,
            Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::error::EAGAIN => break,
            Err(e) => panic!("decode error: {e}"),
        }
    };
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() == index => {
                decoder.send_packet(&packet).unwrap();
                drain(&mut decoder, &mut pcm);
            }
            Ok(()) => {}
            Err(ffmpeg_next::Error::Eof) => break,
            Err(e) => panic!("demux error: {e}"),
        }
    }
    decoder.send_eof().unwrap();
    drain(&mut decoder, &mut pcm);
    pcm
}

#[test]
fn flac_retains_exact_samples_including_short_tail() {
    let dir = tempfile::tempdir().unwrap();
    for samples in [1, 37, 1025, 47999] {
        let input = dir.path().join(format!("input-{samples}.wav"));
        let output = dir.path().join(format!("output-{samples}.flac"));
        let expected = wav(&input, 2, samples, 44100);
        let report = encode(&config(&input, output.clone(), Codec::Flac)).unwrap();
        assert_eq!(report.decoded_samples, samples as u64);
        assert_eq!(decode_i16(&output, 0), expected);
    }
}

#[test]
fn dual_contains_opus_then_flac_and_exact_flac_samples() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    let output = dir.path().join("dual.mka");
    let expected = wav(&input, 2, 7777, 44100);
    let report = encode(&config(&input, output.clone(), Codec::Dual)).unwrap();
    assert_eq!(report.tracks, 2);
    let context = format::input(&output).unwrap();
    assert_eq!(context.stream(0).unwrap().parameters().id(), codec::Id::OPUS);
    assert_eq!(context.stream(1).unwrap().parameters().id(), codec::Id::FLAC);
    assert_eq!(context.stream(0).unwrap().metadata().get("title"), Some("Opus (128k)"));
    assert_eq!(
        context.stream(1).unwrap().metadata().get("title"),
        Some("FLAC (Lossless)")
    );
    assert_eq!(decode_i16(&output, 1), expected);
}

#[test]
fn opus_handles_resampling_and_final_partial_frame() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wav");
    let output = dir.path().join("output.opus");
    wav(&input, 1, 1025, 44100);
    encode(&config(&input, output.clone(), Codec::Opus)).unwrap();
    let context = format::input(&output).unwrap();
    assert_eq!(context.stream(0).unwrap().parameters().id(), codec::Id::OPUS);
    assert!(fs::metadata(output).unwrap().len() > 0);
}

#[test]
fn existing_output_and_missing_track_leave_files_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    let output = dir.path().join("existing.flac");
    wav(&input, 1, 4096, 48000);
    fs::write(&output, b"keep me").unwrap();
    assert!(encode(&config(&input, output.clone(), Codec::Flac)).is_err());
    assert_eq!(fs::read(&output).unwrap(), b"keep me");
    let missing_output = dir.path().join("missing.flac");
    let mut settings = config(&input, missing_output.clone(), Codec::Flac);
    settings.audio_stream = 1;
    assert!(encode(&settings).is_err());
    assert!(!missing_output.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn truehd_stereo_uses_native_encoder() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    let output = dir.path().join("truehd.mka");
    for samples in [640, 641, 1025, 4800] {
        let expected = wav(&input, 2, samples, 48000);
        encode(&config(&input, output.clone(), Codec::Truehd)).unwrap();
        let context = format::input(&output).unwrap();
        assert_eq!(context.stream(0).unwrap().parameters().id(), codec::Id::TRUEHD);
        let actual = decode_i16(&output, 0);
        assert_eq!(actual.len(), expected.len(), "TrueHD sample count for {samples}");
        assert!(actual == expected, "TrueHD PCM mismatch for {samples}");
        drop(context);
        fs::remove_file(&output).unwrap();
    }
}

#[test]
fn truehd_rejects_too_short_input_and_unsupported_rates_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.wav");
    let output = dir.path().join("truehd.mka");
    for (samples, rate) in [(37, 48000), (4096, 22050)] {
        wav(&input, 2, samples, rate);
        assert!(encode(&config(&input, output.clone(), Codec::Truehd)).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
