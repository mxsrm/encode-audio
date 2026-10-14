#!/usr/bin/env python3
"""Independent integration oracle; only this test harness launches FFmpeg.

Run after `cargo build`: python3 tests/verify_media.py target/debug/encode-audio
Requires ffmpeg and ffprobe on PATH. All fixtures live in a temporary directory.
"""

import json
import math
import pathlib
import subprocess
import sys
import tempfile
import wave


def command(*args):
    result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise AssertionError(f"{args[0]} failed: {result.stderr.decode(errors='replace')}")
    return result.stdout


def fixture(path, channels, bits=16, samples=7777, rate=48000):
    pcm = bytearray()
    for i in range(samples):
        for channel in range(channels):
            value = int(math.sin(i * (channel + 1) * 0.02) * ((1 << (bits - 2)) - 1))
            pcm.extend(value.to_bytes(bits // 8, "little", signed=True))
    with wave.open(str(path), "wb") as output:
        output.setnchannels(channels)
        output.setsampwidth(bits // 8)
        output.setframerate(rate)
        output.writeframes(pcm)
    return bytes(pcm)


def decode(path, track=0, bits=16):
    return command("ffmpeg", "-v", "error", "-i", str(path), "-map", f"0:a:{track}",
                   "-f", f"s{bits}le", "-c:a", f"pcm_s{bits}le", "-")


def streams(path):
    return json.loads(command("ffprobe", "-v", "error", "-show_streams", "-of", "json", str(path)))["streams"]


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory() as directory:
        root = pathlib.Path(directory)
        checked = 0
        for channels, bits, samples, rate in [(1, 16, 1025, 48000), (2, 24, 7777, 48000),
                                              (6, 24, 7777, 48000), (8, 16, 7777, 48000),
                                              (2, 16, 1025, 44100)]:
            source = root / f"source-{channels}-{bits}.wav"
            expected = fixture(source, channels, bits, samples, rate)
            for codec in ["flac", "opus", "dual", "truehd"]:
                if codec == "truehd" and channels == 8:
                    continue
                output = root / f"result-{channels}-{bits}-{codec}"
                command(binary, "-i", str(source), "-o", str(output), "-c", codec)
                info = streams(output)
                expected_codecs = ["opus", "flac"] if codec == "dual" else [codec]
                assert [s["codec_name"] for s in info] == expected_codecs
                assert all(s["channels"] == channels for s in info)
                if codec in ["flac", "truehd", "dual"]:
                    actual = decode(output, 1 if codec == "dual" else 0, bits)
                    assert len(actual) == len(expected), (codec, channels, len(actual), len(expected))
                    assert actual == expected, (codec, channels, "PCM mismatch")
                if codec in ["opus", "dual"]:
                    actual = decode(output, 0)
                    converted_samples = (samples * 48000 + rate - 1) // rate
                    assert len(actual) == converted_samples * channels * 2, (codec, "Opus duration mismatch")
                checked += 1

        first, second, movie = root / "first.wav", root / "second.wav", root / "movie.mkv"
        fixture(first, 1, samples=4096)
        expected = fixture(second, 2, samples=7777)
        command("ffmpeg", "-v", "error", "-i", str(first), "-i", str(second),
                "-f", "lavfi", "-i", "color=size=16x16:rate=1:duration=1",
                "-map", "2:v:0", "-map", "0:a:0", "-map", "1:a:0",
                "-c:v", "ffv1", "-c:a", "pcm_s16le", str(movie))
        selected = root / "selected.flac"
        command(binary, "-i", str(movie), "-c", "flac", "-s", "1", "-o", str(selected))
        assert decode(selected) == expected, "audio-relative selection failed"
        checked += 1

        side = root / "side.mka"
        source = root / "source-6-24.wav"
        command("ffmpeg", "-v", "error", "-i", str(source), "-channel_layout", "5.1(side)",
                "-c:a", "pcm_s24le", str(side))
        for codec in ["opus", "flac", "truehd", "dual"]:
            output = root / f"side-{codec}.mka"
            command(binary, "-i", str(side), "-c", codec, "-o", str(output))
            assert all(s["channels"] == 6 for s in streams(output))
            checked += 1

        source = root / "7.1.wav"
        fixture(source, 8)
        rejected = root / "rejected.mka"
        result = subprocess.run([binary, "-i", str(source), "-c", "truehd", "-o", str(rejected)],
                                capture_output=True)
        assert result.returncode != 0 and not rejected.exists(), "unsupported TrueHD layout must fail cleanly"
        assert not list(root.glob(".encode-audio-*")), "failed job left temporary output"
        checked += 1
        print(f"Passed {checked} independent media checks")


if __name__ == "__main__":
    main()

