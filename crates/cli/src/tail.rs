//! The `tail` verb: frames off the wire, one line each, and a line for every
//! gap in their numbering.
//!
//! A frame is a little-endian `u32` length and then one `Envelope`. The
//! broker numbers `seq` per connection, from one, before it decides whether a
//! record under pressure stays, so a `seq` that skips means records were
//! dropped and nothing else. That is what this prints, and it is the first
//! thing a person at a live install can see of the drop policy.
//!
//! The envelope is decoded through a stock protobuf library and the record
//! inside it is not decoded at all: with no schema loaded, its type URL is
//! all `tail` knows of it, and the type URL is the topic.

use std::io::{self, Read, Write};

use prost::Message;

/// What protobuf runtimes put in front of a type name in an `Any`. Stripped
/// from the printed topic, because every record carries it.
const TYPE_URL_PREFIX: &str = "type.googleapis.com/";

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

/// What a run saw, for the closing line and for the tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// Frames printed.
    pub frames: u64,
    /// Places where `seq` skipped.
    pub gaps: u64,
    /// Records the skips add up to.
    pub dropped: u64,
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

/// Print each frame from `reader` to `out` until the stream ends, with a
/// line before any frame whose `seq` skips past the one before it.
///
/// Each line is flushed as it is written, so a person watching sees a frame
/// when it arrives rather than when a buffer fills.
pub fn run(mut reader: impl Read, mut out: impl Write) -> io::Result<Summary> {
    let mut summary = Summary::default();
    let mut last_seq = 0;

    while let Some(envelope) = read_frame(&mut reader)? {
        if envelope.seq > last_seq + 1 {
            let missing = envelope.seq - last_seq - 1;
            if last_seq == 0 {
                writeln!(
                    out,
                    "gap: {missing} records dropped before seq {}",
                    envelope.seq
                )?;
            } else {
                writeln!(
                    out,
                    "gap: {missing} records dropped between seq {last_seq} and {}",
                    envelope.seq
                )?;
            }
            summary.gaps += 1;
            summary.dropped += missing;
        } else if envelope.seq <= last_seq {
            writeln!(out, "seq {} after {last_seq}: out of order", envelope.seq)?;
        }

        write_frame_line(&mut out, &envelope)?;
        out.flush()?;
        last_seq = envelope.seq;
        summary.frames += 1;
    }

    Ok(summary)
}

/// One line per frame: `seq`, the topic and the payload's size, then the
/// epoch and mission time only when the frame carries them.
fn write_frame_line(out: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    write!(out, "seq={}", envelope.seq)?;
    match &envelope.payload {
        Some(any) => {
            let topic = any
                .type_url
                .strip_prefix(TYPE_URL_PREFIX)
                .unwrap_or(&any.type_url);
            write!(out, " topic={topic} bytes={}", any.value.len())?;
        }
        None => write!(out, " topic=- bytes=0")?,
    }
    if let Some(epoch) = envelope.epoch {
        write!(out, " epoch={epoch}")?;
    }
    if let Some(mission_time) = envelope.mission_time {
        write!(out, " mission_time={mission_time}")?;
    }
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcsbridge_broker::encode::Encoder;
    use dcsbridge_broker::fanout::{Commit, Writer};
    use dcsbridge_broker::transport::{Listener, Record};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const TOPIC: &str = "dcs.builtin.UnitDestroyed";

    /// A frame as a stock encoder writes it, for a stream built by hand.
    fn frame(seq: u64) -> Vec<u8> {
        let envelope = Envelope {
            seq,
            epoch: None,
            mission_time: None,
            payload: Some(prost_types::Any {
                type_url: format!("{TYPE_URL_PREFIX}{TOPIC}"),
                value: vec![0x08, 0x2a],
            }),
        };
        let body = envelope.encode_to_vec();
        let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    /// The gap line's arithmetic: `seq` 1, 2, 5 is two records missing
    /// between 2 and 5, and the frames on either side print as frames.
    #[test]
    fn a_skip_in_seq_prints_the_records_missing() {
        let mut stream = Vec::new();
        for seq in [1, 2, 5] {
            stream.extend(frame(seq));
        }

        let mut out = Vec::new();
        let summary = run(&stream[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert_eq!(
            summary,
            Summary {
                frames: 3,
                gaps: 1,
                dropped: 2
            }
        );
        assert_eq!(
            out,
            "seq=1 topic=dcs.builtin.UnitDestroyed bytes=2\n\
             seq=2 topic=dcs.builtin.UnitDestroyed bytes=2\n\
             gap: 2 records dropped between seq 2 and 5\n\
             seq=5 topic=dcs.builtin.UnitDestroyed bytes=2\n"
        );
    }

    /// A stream that ends between frames is a closed connection; one that
    /// ends inside a frame lost bytes, and says so.
    #[test]
    fn an_end_of_stream_is_clean_only_between_frames() {
        let whole = frame(1);
        assert!(read_frame(&mut &whole[..]).unwrap().is_some());
        assert!(read_frame(&mut &whole[..0]).unwrap().is_none());

        let cut = &whole[..whole.len() - 1];
        let error = read_frame(&mut &cut[..]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        let error = read_frame(&mut &whole[..2]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    /// A record on [`TOPIC`] carrying `bytes` of string in field 1.
    fn record(bytes: usize) -> Record {
        let mut e = Encoder::with_capacity(bytes + 64);
        e.begin(TOPIC.as_bytes());
        e.string(1, &vec![b'x'; bytes]).unwrap();
        Arc::from(e.commit().unwrap())
    }

    fn client(addr: SocketAddr) -> TcpStream {
        TcpStream::connect(addr).expect("the listener accepts")
    }

    /// Commit small records until `client` has a frame waiting. A record
    /// committed before the writer thread knows the connection is not
    /// delivered to it, and nothing outside that thread says when that is.
    fn warm_up(commit: &mut Commit<Record>, client: &TcpStream) {
        let deadline = Instant::now() + Duration::from_secs(30);
        client
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("a read timeout is set");
        let mut length = [0u8; 4];
        loop {
            assert!(Instant::now() < deadline, "no frame arrived");
            drop(commit.push(record(1)));
            if matches!(client.peek(&mut length), Ok(4)) {
                return;
            }
        }
    }

    /// Everything `client` receives until it has been quiet for a while.
    fn drain(client: &mut TcpStream) -> Vec<u8> {
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("a read timeout is set");
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 1 << 16];
        loop {
            match client.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("the read failed: {error}"),
            }
        }
        bytes
    }

    /// The forced drop, observed the way an operator observes it: a consumer
    /// stops reading, its ring evicts behind the blocked socket, and when it
    /// reads again `tail` prints a gap where the evicted records were.
    ///
    /// The ring holds four records and the burst is far larger than the
    /// loopback socket can buffer, so the connection's thread blocks on the
    /// socket while the writer thread pushes the rest of the burst past it.
    #[test]
    fn a_stalled_consumer_sees_its_evictions_as_a_gap() {
        let (writer, mut commit, connections) = Writer::spawn(4096);
        let greeting = dcsbridge_broker::bridge().handshake();
        let listener =
            Listener::spawn("127.0.0.1:0", connections, 4, move || greeting.encode()).unwrap();
        let mut client = client(listener.local_addr());
        warm_up(&mut commit, &client);

        let big = record(64 << 10);
        for _ in 0..512 {
            drop(commit.push(Arc::clone(&big)));
        }

        let bytes = drain(&mut client);
        drop(listener);
        drop(writer);

        let mut out = Vec::new();
        let summary = run(&bytes[..], &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(summary.frames >= 1, "no frame was read");
        assert!(summary.gaps >= 1, "the stall left no gap:\n{out}");
        assert!(summary.dropped >= 1, "a gap dropped nothing:\n{out}");
        assert!(
            out.contains("gap: ") && out.contains(" records dropped between seq "),
            "the gap was not printed:\n{out}"
        );
        assert!(
            out.contains(&format!(" topic={TOPIC} bytes=")),
            "the topic was not printed without its prefix:\n{out}"
        );
        assert!(!out.contains("out of order"), "seq went backwards:\n{out}");

        let seqs: Vec<u64> = out
            .lines()
            .filter_map(|line| line.strip_prefix("seq="))
            .map(|rest| rest.split(' ').next().unwrap().parse().unwrap())
            .collect();
        assert!(
            seqs.windows(2).all(|pair| pair[0] < pair[1]),
            "seq did not rise strictly:\n{out}"
        );
    }
}
