//! The wire format, as every verb reads and writes it.
//!
//! A frame is a little-endian `u32` length and then one `Envelope`. The
//! envelope is decoded through a stock protobuf library and the record
//! inside it stays opaque: its type URL is the topic, and whoever the topic
//! is for decodes the value.

use std::io::{self, Read};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use prost::Message;

/// What protobuf runtimes put in front of a type name in an `Any`. Stripped
/// from a printed topic, because every record carries it.
pub const TYPE_URL_PREFIX: &str = "type.googleapis.com/";

/// The most bytes a frame may claim before the length is read as garbage
/// rather than obeyed. The bridge's own frame cap is smaller.
const FRAME_MAX: u32 = 16 << 20;

/// `dcs.bridge.Envelope`, as `proto/dcs/bridge/bridge.proto` numbers it.
///
/// The payload is an `Any` whose value stays opaque here.
#[derive(Clone, PartialEq, Message)]
pub struct Envelope {
    /// This connection's number for the frame, from one.
    #[prost(uint64, tag = "1")]
    pub seq: u64,
    /// Absent outside an epoch.
    #[prost(uint32, optional, tag = "2")]
    pub epoch: Option<u32>,
    /// Absent while the sim is not running.
    #[prost(double, optional, tag = "3")]
    pub mission_time: Option<f64>,
    /// The record, behind its type URL.
    #[prost(message, optional, tag = "4")]
    pub payload: Option<prost_types::Any>,
}

impl Envelope {
    /// The topic: the payload's type URL without its prefix, or `None` for
    /// a frame with no payload.
    pub fn topic(&self) -> Option<&str> {
        let any = self.payload.as_ref()?;
        Some(
            any.type_url
                .strip_prefix(TYPE_URL_PREFIX)
                .unwrap_or(&any.type_url),
        )
    }
}

/// One frame carrying `value` on `topic`, numbered `seq`, with no epoch and
/// no mission time: what a consumer sends, and what a test builds.
pub fn frame(seq: u64, topic: &str, value: Vec<u8>) -> Vec<u8> {
    let envelope = Envelope {
        seq,
        epoch: None,
        mission_time: None,
        payload: Some(prost_types::Any {
            type_url: format!("{TYPE_URL_PREFIX}{topic}"),
            value,
        }),
    };
    let body = envelope.encode_to_vec();
    let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
    bytes.extend(body);
    bytes
}

/// Read one frame, or `None` at a clean end of stream.
///
/// An end of stream inside a frame is an error, because the bridge closes a
/// connection between frames and a cut mid-frame means bytes were lost.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Envelope>> {
    let mut length = [0u8; 4];
    match fill(reader, &mut length)? {
        0 => return Ok(None),
        4 => {}
        n => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("the stream ended {n} bytes into a frame's length"),
            ));
        }
    }
    let length = u32::from_le_bytes(length);
    if length > FRAME_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame claims {length} bytes, which is not a frame"),
        ));
    }

    let mut body = vec![0u8; length as usize];
    let got = fill(reader, &mut body)?;
    if got != body.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("the stream ended {got} bytes into a {length}-byte frame"),
        ));
    }

    Envelope::decode(&body[..])
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// A socket read against a wall-clock deadline.
///
/// A socket's own read timeout bounds one read, so a peer that sends a byte
/// every few seconds passes it on every read and is never caught by it.
/// Each read here is armed with what is left of the deadline, and once
/// nothing is left the read fails as timed out without touching the socket.
pub struct Deadline {
    stream: TcpStream,
    until: Instant,
}

impl Deadline {
    /// Read `stream` for at most `wait` from now.
    pub fn new(stream: TcpStream, wait: Duration) -> Self {
        Self {
            stream,
            until: Instant::now() + wait,
        }
    }
}

impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}

/// Fill `buf` from `reader`, returning how many bytes arrived before the
/// stream ended. `read_exact` cannot tell an end of stream at a frame
/// boundary from one inside a frame, and the two mean different things.
fn fill(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream that ends between frames is a closed connection; one that
    /// ends inside a frame lost bytes, and says so.
    #[test]
    fn an_end_of_stream_is_clean_only_between_frames() {
        let whole = frame(1, "dcs.builtin.UnitDestroyed", vec![0x08, 0x2a]);
        let envelope = read_frame(&mut &whole[..]).unwrap().unwrap();
        assert_eq!(envelope.topic(), Some("dcs.builtin.UnitDestroyed"));
        assert!(read_frame(&mut &whole[..0]).unwrap().is_none());

        let cut = &whole[..whole.len() - 1];
        let error = read_frame(&mut &cut[..]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        let error = read_frame(&mut &whole[..2]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    /// A peer that sends a byte at a time, each inside the socket's own
    /// timeout, is still caught by the deadline: the frame never completes
    /// and the read fails as timed out once the wait is spent.
    #[test]
    fn a_trickled_frame_does_not_outlive_the_deadline() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let trickle = thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            for _ in 0..40 {
                if peer.write_all(&[0]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });

        let wait = Duration::from_millis(400);
        let mut reader = Deadline::new(TcpStream::connect(addr).unwrap(), wait);
        let started = Instant::now();
        let error = read_frame(&mut reader).unwrap_err();
        let took = started.elapsed();
        drop(reader);
        trickle.join().unwrap();

        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "the read failed for another reason: {error}"
        );
        assert!(took >= wait, "the read gave up early, after {took:?}");
        assert!(
            took < wait * 4,
            "the trickle held the read past the deadline, for {took:?}"
        );
    }
}
