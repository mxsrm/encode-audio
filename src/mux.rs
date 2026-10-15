//! Muxer adapter for payload and side-data-only packets.

use ffmpeg_next::{Packet, format, packet::Mut};

/// FFmpeg's FLAC encoder emits a final packet containing updated STREAMINFO
/// (including the MD5 and sample count) but no audio payload. The high-level
/// `write_interleaved` helper rejects empty payloads before FFmpeg sees their
/// side data. Use the underlying binding for this specific packet shape.
pub(crate) fn write(packet: &mut Packet, output: &mut format::context::Output) -> Result<(), ffmpeg_next::Error> {
    if packet.size() > 0 {
        return packet.write_interleaved(output);
    }
    // SAFETY: both objects own live FFmpeg contexts, borrowed exclusively here.
    // av_interleaved_write_frame accepts a valid zero-size AVPacket with side
    // data, and may consume its references; Packet's destructor handles that.
    let result = unsafe { ffmpeg_next::ffi::av_interleaved_write_frame(output.as_mut_ptr(), packet.as_mut_ptr()) };
    if result < 0 {
        Err(ffmpeg_next::Error::from(result))
    } else {
        Ok(())
    }
}
