//! The `record` verb: every frame a bridge sends, appended to a file as the
//! wire carried it.
//!
//! A capture is the connection's bytes from the handshake on, the
//! `AuthResult` included, so `replay` can serve it to a consumer that then
//! reads what a live bridge would have sent, and `tail` reads it from `seq`
//! 1 with no gap line. Nothing is re-encoded: the CLI's `Envelope` declares
//! only the fields its verbs read, and a frame written back through it
//! would lose the rest. ADR 0033 holds the argument.

use std::io::{self, Read, Write};

use prost::Message;

use crate::wire::{Envelope, auth_result, read_frame_bytes};

/// What a run wrote, for the closing line and for the tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// Frames written.
    pub frames: u64,
    /// Bytes written, length prefixes included.
    pub bytes: u64,
    /// The bridge answered the token with an error, and closed after it.
    pub refused: bool,
}

/// Copy each frame from `reader` to `sink` until the stream ends.
///
/// Each frame is flushed as it is written, so a run that is stopped leaves
/// whole frames behind it and nothing else. An end of stream inside a frame
/// is an error, after every frame before it has been written.
pub fn run(mut reader: impl Read, mut sink: impl Write) -> io::Result<Summary> {
    let mut summary = Summary::default();
    while let Some(bytes) = read_frame_bytes(&mut reader)? {
        sink.write_all(&bytes)?;
        sink.flush()?;
        summary.frames += 1;
        summary.bytes += bytes.len() as u64;
        // The one frame read into is the token's answer, because a refusal
        // is an exit code; a frame that does not decode is still written,
        // since the capture is the bytes and not their reading.
        if let Ok(envelope) = Envelope::decode(&bytes[4..])
            && let Some(result) = auth_result(&envelope)
        {
            summary.refused |= !result.ok;
        }
    }
    Ok(summary)
}

/// Where an existing capture's whole frames end: `Ok(length)` when the
/// capture ends at a frame boundary, `Err(offset)` when it ends inside a
/// frame, with `offset` the end of the last whole frame.
///
/// A `record` killed mid-write leaves a partial frame, and a run that
/// appended after it would have that frame's length prefix swallow the
/// next session's first bytes, so every frame after the join would read as
/// garbage. Appending onto one is refused instead.
pub fn whole_frames(mut reader: impl Read) -> io::Result<Result<u64, u64>> {
    let mut whole = 0;
    loop {
        match read_frame_bytes(&mut reader) {
            Ok(Some(bytes)) => whole += bytes.len() as u64,
            Ok(None) => return Ok(Ok(whole)),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(Err(whole));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{self, AuthResult};
    use dcsbridge_topic as topic;

    const TOPIC: &str = "dcsbridge.builtin.sim.UnitDestroyed";

    /// Frames of three sizes, numbered from one.
    fn frames() -> Vec<Vec<u8>> {
        [1usize, 200, 7000]
            .into_iter()
            .enumerate()
            .map(|(i, bytes)| wire::frame(i as u64 + 1, TOPIC, vec![b'x'; bytes]))
            .collect()
    }

    /// A sink that keeps what was flushed apart from what was written, so a
    /// test can see that every frame reached the file before the next read.
    #[derive(Default)]
    struct Flushed {
        pending: Vec<u8>,
        flushed: Vec<u8>,
    }

    impl Write for Flushed {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.pending.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed.append(&mut self.pending);
            Ok(())
        }
    }

    /// What comes out is what went in, byte for byte, and the summary
    /// counts the frames and the bytes.
    #[test]
    fn a_capture_is_the_stream_byte_for_byte() {
        let stream: Vec<u8> = frames().concat();
        let mut sink = Flushed::default();
        let summary = run(&stream[..], &mut sink).unwrap();
        assert_eq!(sink.flushed, stream);
        assert!(sink.pending.is_empty(), "the last frame was not flushed");
        assert_eq!(
            summary,
            Summary {
                frames: 3,
                bytes: stream.len() as u64,
                refused: false,
            }
        );
    }

    /// A refused token is reported, and its answer is in the capture like
    /// any other frame.
    #[test]
    fn a_refused_token_is_reported_and_kept() {
        let refusal = wire::frame(
            2,
            topic::AUTH_RESULT,
            AuthResult {
                ok: false,
                error: 1,
            }
            .encode_to_vec(),
        );
        let mut sink = Vec::new();
        let summary = run(&refusal[..], &mut sink).unwrap();
        assert!(summary.refused);
        assert_eq!(sink, refusal);

        let accepted = wire::frame(
            2,
            topic::AUTH_RESULT,
            AuthResult { ok: true, error: 0 }.encode_to_vec(),
        );
        let summary = run(&accepted[..], &mut Vec::new()).unwrap();
        assert!(!summary.refused);
    }

    /// A stream cut inside a frame is an error, after the whole frames
    /// before it were written and flushed; the partial frame is not.
    #[test]
    fn a_cut_stream_leaves_the_whole_frames_flushed() {
        let frames = frames();
        let whole: Vec<u8> = frames[..2].concat();
        let mut stream = whole.clone();
        stream.extend(&frames[2][..frames[2].len() / 2]);

        let mut sink = Flushed::default();
        let error = run(&stream[..], &mut sink).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(sink.flushed, whole);
        assert!(sink.pending.is_empty());
    }

    /// An existing capture is whole when it ends at a frame boundary, and
    /// one that ends inside a frame reports where its last whole frame ended.
    #[test]
    fn an_existing_capture_is_checked_for_a_frame_boundary() {
        let frames = frames();
        let whole: Vec<u8> = frames.concat();
        assert_eq!(whole_frames(&whole[..]).unwrap(), Ok(whole.len() as u64));
        assert_eq!(whole_frames(&[][..]).unwrap(), Ok(0));

        let boundary = frames[0].len() + frames[1].len();
        let cut = &whole[..boundary + 3];
        assert_eq!(whole_frames(cut).unwrap(), Err(boundary as u64));

        let garbage = u32::MAX.to_le_bytes();
        assert_eq!(
            whole_frames(&garbage[..]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
