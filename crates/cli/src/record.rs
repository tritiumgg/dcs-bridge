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
    use crate::loopback::{READ, TOPIC, client, record};
    use crate::replay;
    use crate::tail;
    use crate::wire::{self, AuthResult, auth_frame, read_frame};
    use dcsbridge_broker::fanout::{Capacities, Class, Writer};
    use dcsbridge_broker::transport::Listener;
    use dcsbridge_topic as topic;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

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

    /// A sink shared with the thread `run` writes on, so the test can watch
    /// the capture grow.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Shared {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Wait until `sink` holds a frame whose payload is `bytes` long, or
    /// fail after a while.
    fn wait_for_payload(sink: &Shared, bytes: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !payload_sizes(&sink.bytes()).contains(&bytes) {
            assert!(
                Instant::now() < deadline,
                "no frame of {bytes} bytes arrived"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The payload size of each whole frame in `capture`.
    fn payload_sizes(capture: &[u8]) -> Vec<usize> {
        let mut reader = capture;
        let mut sizes = Vec::new();
        while let Ok(Some(envelope)) = read_frame(&mut reader) {
            sizes.push(envelope.payload.map_or(0, |any| any.value.len()));
        }
        sizes
    }

    /// The whole round trip, with no DCS and no bridge on the replay side:
    /// a session `record` captured off the broker's own listener,
    /// handshake and token answer included, is served by `replay` to a
    /// consumer that reads it through `tail` frame for frame, with no gap.
    /// The recorded records have distinct sizes, and each size is read
    /// back, so a frame missing from the capture or the replay would show.
    #[test]
    fn a_session_recorded_off_the_bridge_replays_frame_for_frame() {
        const SIZES: [usize; 4] = [16, 300, 5000, 20000];
        let (writer, mut commit, connections) = Writer::spawn(4096);
        let answers = Arc::new(dcsbridge_broker::state::Global);
        let listener =
            Listener::spawn("127.0.0.1:0", connections, Capacities::each(64), answers).unwrap();
        // Nothing is read off the socket here: the handshake and the token
        // answer are `record`'s to read, so the capture starts at seq 1.
        let socket = client(listener.local_addr());
        let control = socket.try_clone().unwrap();
        let sink = Shared::default();
        let recording = thread::spawn({
            let sink = sink.clone();
            move || run(&socket, sink)
        });

        // A record committed before the writer thread knows the connection
        // has authenticated is passed over, and nothing outside that thread
        // says when that is; a third frame in the capture does.
        let deadline = Instant::now() + Duration::from_secs(30);
        while payload_sizes(&sink.bytes()).len() < 3 {
            assert!(Instant::now() < deadline, "no record reached the capture");
            drop(commit.push(READ, Class::Durable, record(1)));
            thread::sleep(Duration::from_millis(5));
        }
        // The payload the wire carries is the record's `Any` value, a few
        // bytes over the string inside it.
        let sizes: Vec<usize> = SIZES
            .into_iter()
            .map(|bytes| {
                let record = record(bytes);
                drop(commit.push(READ, Class::Durable, Arc::clone(&record)));
                let tail = Envelope::decode(&record[..]).unwrap();
                tail.payload.unwrap().value.len()
            })
            .collect();
        wait_for_payload(&sink, sizes[SIZES.len() - 1]);

        // A shutdown of the consumer's own socket ends the stream between
        // frames, which `run` reads as a clean close.
        control.shutdown(Shutdown::Both).unwrap();
        let recorded = recording.join().unwrap().unwrap();
        drop(listener);
        drop(writer);
        let capture = sink.bytes();
        assert!(!recorded.refused, "the token was refused");
        assert_eq!(recorded.bytes, capture.len() as u64);
        assert_eq!(recorded.frames, payload_sizes(&capture).len() as u64);

        let replay_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = replay_listener.local_addr().unwrap();
        let replaying = thread::spawn({
            let capture = capture.clone();
            move || replay::run(&replay_listener, &capture[..])
        });
        let mut consumer = TcpStream::connect(addr).unwrap();
        consumer
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        consumer.write_all(&auth_frame("anything")).unwrap();
        let mut replayed = Vec::new();
        consumer.read_to_end(&mut replayed).unwrap();
        drop(consumer);
        replaying.join().unwrap().unwrap();
        assert_eq!(replayed, capture, "the consumer did not read the capture");

        let mut out = Vec::new();
        let summary = tail::run(&replayed[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(!summary.refused, "{out}");
        assert_eq!(summary.frames, recorded.frames, "{out}");
        assert_eq!(summary.gaps, 0, "{out}");
        assert!(!out.contains("out of order"), "{out}");
        let lines: Vec<&str> = out.lines().collect();
        assert!(
            lines[0].starts_with("seq=1 ")
                && lines[0].contains(&format!(" topic={} ", topic::HANDSHAKE)),
            "the handshake is not first:\n{out}"
        );
        assert!(
            lines[1].starts_with("seq=2 ") && lines[1].ends_with(" ok=true"),
            "the token answer is not second:\n{out}"
        );
        for bytes in sizes {
            assert!(
                out.contains(&format!(" topic={TOPIC} bytes={bytes}\n")),
                "the {bytes}-byte record did not come back:\n{out}"
            );
        }
    }
}
