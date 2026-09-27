//! The `replay` verb: a capture served to one consumer, frame for frame,
//! with no bridge running.
//!
//! The consumer connects as it would to a bridge and sends its `Auth`. No
//! token table exists here, so whatever it sends is read and discarded and
//! the recorded `AuthResult` is what answers it. Frames go out as fast as
//! the consumer reads them: a capture carries no wall clock, and mission
//! time is absent outside an epoch. ADR 0034 holds the argument.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::wire::read_frame_bytes;

/// What a run served, for the closing line and for the tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// Frames written to the consumer.
    pub frames: u64,
    /// Bytes written, length prefixes included.
    pub bytes: u64,
    /// Bytes the consumer sent, read and thrown away.
    pub discarded: u64,
}

/// How long the inbound reader waits on one read before it looks at whether
/// the capture has been served.
const READ_SLICE: Duration = Duration::from_millis(200);

/// How long after the last frame the close waits for the consumer to close
/// its side. A consumer still working through its receive buffer may write
/// in that time, and a close before it does draws the reset the drain is
/// there to avoid; one that neither closes nor goes quiet past this is
/// closed on regardless, so a consumer that never closes cannot hold the
/// process.
const DRAIN_MAX: Duration = Duration::from_secs(5);

/// Accept one consumer on `listener`, serve `capture` to it, and return.
pub fn run(listener: &TcpListener, capture: impl Read) -> io::Result<Summary> {
    let (consumer, _) = listener.accept()?;
    serve(capture, consumer)
}

/// Write each frame of `capture` to `consumer`, whole, then close.
///
/// The consumer's own bytes are read and discarded from the start, on a
/// second thread, so a consumer that writes while it reads is never stalled
/// against a full buffer. The close is a write-side shutdown followed by a
/// read to the consumer's end of stream: a close over unread inbound bytes
/// is answered with a reset on Linux, and a reset can take the frames still
/// in the consumer's receive buffer with it. A capture that ends inside a
/// frame is an error, after every whole frame before it has been sent.
pub fn serve(capture: impl Read, consumer: TcpStream) -> io::Result<Summary> {
    let inbound = consumer.try_clone()?;
    inbound.set_read_timeout(Some(READ_SLICE))?;
    let served = Arc::new(AtomicBool::new(false));
    let discarded = Arc::new(AtomicU64::new(0));
    let reader = thread::spawn({
        let served = Arc::clone(&served);
        let discarded = Arc::clone(&discarded);
        move || discard(inbound, &served, &discarded)
    });

    let mut consumer = consumer;
    let result = write_frames(capture, &mut consumer);
    let _ = consumer.shutdown(Shutdown::Write);
    served.store(true, Ordering::SeqCst);
    let _ = reader.join();

    let (frames, bytes) = result?;
    Ok(Summary {
        frames,
        bytes,
        discarded: discarded.load(Ordering::SeqCst),
    })
}

/// Write each frame of `capture` to `out`, whole, and count them.
pub fn write_frames(mut capture: impl Read, out: &mut impl Write) -> io::Result<(u64, u64)> {
    let mut frames = 0;
    let mut bytes = 0;
    while let Some(frame) = read_frame_bytes(&mut capture)? {
        out.write_all(&frame)?;
        frames += 1;
        bytes += frame.len() as u64;
    }
    out.flush()?;
    Ok((frames, bytes))
}

/// Read `inbound` until the consumer closes its side, counting the bytes,
/// or until the capture has been served and [`DRAIN_MAX`] has passed.
fn discard(mut inbound: TcpStream, served: &AtomicBool, discarded: &AtomicU64) {
    let mut chunk = [0u8; 1 << 12];
    let mut served_at = None;
    loop {
        if served.load(Ordering::SeqCst) {
            let since = *served_at.get_or_insert_with(Instant::now);
            if since.elapsed() >= DRAIN_MAX {
                return;
            }
        }
        match inbound.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                discarded.fetch_add(n as u64, Ordering::SeqCst);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tail;
    use crate::wire::{self, Envelope, auth_frame};
    use prost::Message;

    const TOPIC: &str = "dcsbridge.builtin.sim.UnitDestroyed";

    /// Three frames, the second carrying an epoch and a mission time, as a
    /// frame the bridge sent mid-mission does.
    fn capture() -> Vec<Vec<u8>> {
        let mid = Envelope {
            seq: 2,
            epoch: Some(2),
            mission_time: Some(1.5),
            payload: Some(prost_types::Any {
                type_url: format!("{}{TOPIC}", wire::TYPE_URL_PREFIX),
                value: vec![b'x'; 200],
            }),
        }
        .encode_to_vec();
        let mut second = (mid.len() as u32).to_le_bytes().to_vec();
        second.extend(mid);
        vec![
            wire::frame(1, TOPIC, vec![b'x'; 1]),
            second,
            wire::frame(3, TOPIC, vec![b'x'; 7000]),
        ]
    }

    /// What goes out is the capture, byte for byte, and a capture that ends
    /// inside a frame sends the whole frames before it and then fails.
    #[test]
    fn the_frames_written_are_the_capture() {
        let frames = capture();
        let whole: Vec<u8> = frames.concat();
        let mut out = Vec::new();
        assert_eq!(
            write_frames(&whole[..], &mut out).unwrap(),
            (3, whole.len() as u64)
        );
        assert_eq!(out, whole);

        let boundary = frames[0].len() + frames[1].len();
        let cut = &whole[..boundary + 5];
        let mut out = Vec::new();
        let error = write_frames(cut, &mut out).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(out, whole[..boundary]);
    }

    /// A consumer that connects and authenticates as it would to a bridge
    /// reads the capture through `tail` with no gap, the epoch and mission
    /// time a frame carried included, and everything it sent was read and
    /// counted, so the close finds nothing unread.
    #[test]
    fn a_consumer_reads_the_capture_through_tail_with_no_gap() {
        let whole: Vec<u8> = capture().concat();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let replay = thread::spawn(move || run(&listener, &whole[..]));

        let mut consumer = TcpStream::connect(addr).unwrap();
        consumer
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let auth = auth_frame("anything");
        consumer.write_all(&auth).unwrap();
        let mut out = Vec::new();
        let summary = tail::run(&consumer, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // The close waits for the consumer's side to close, as `tail`'s
        // exit closes it.
        drop(consumer);
        let served = replay.join().unwrap().unwrap();

        assert_eq!(summary.frames, 3, "{out}");
        assert_eq!(summary.gaps, 0, "{out}");
        assert!(!out.contains("out of order"), "{out}");
        assert!(
            out.contains("seq=2 topic=") && out.contains(" epoch=2 mission_time=1.5\n"),
            "the epoch and mission time did not come through:\n{out}"
        );
        assert_eq!(served.frames, 3);
        assert_eq!(served.bytes, capture().concat().len() as u64);
        assert_eq!(served.discarded, auth.len() as u64);
    }

    /// A consumer that writes only after the whole capture has been sent,
    /// as one working through its receive buffer would, still ends cleanly:
    /// its bytes are read before the close, and its read reaches the end of
    /// stream rather than a reset.
    #[test]
    fn a_consumer_that_writes_late_reads_to_a_clean_end() {
        let whole: Vec<u8> = capture().concat();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let replay = thread::spawn(move || run(&listener, &whole[..]));

        let mut consumer = TcpStream::connect(addr).unwrap();
        consumer
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        thread::sleep(READ_SLICE * 3);
        let auth = auth_frame("anything");
        consumer.write_all(&auth).unwrap();
        let mut read = Vec::new();
        consumer.read_to_end(&mut read).unwrap();
        drop(consumer);
        let served = replay.join().unwrap().unwrap();

        assert_eq!(read, capture().concat());
        assert_eq!(served.discarded, auth.len() as u64);
    }
}
