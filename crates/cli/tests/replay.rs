//! The three verbs as processes: `dcsb record` captures a session off the
//! broker's own listener into a file, `dcsb replay` serves that file, and
//! `dcsb tail` reads it back with no gap. What the in-process tests cannot
//! see is here: the argument wiring, the file opened for append, the
//! `listening on` line a script keys off, the closing lines and the exit
//! codes.
//!
//! An integration test is its own process, so the token table it sets is
//! not the one the unit tests set in parallel.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use dcsbridge_broker::encode::Encoder;
use dcsbridge_broker::fanout::{Capacities, Class, Writer};
use dcsbridge_broker::registry::Capability;
use dcsbridge_broker::state::Token;
use dcsbridge_broker::transport::{Listener, Record};

const TOPIC: &str = "dcsbridge.builtin.sim.UnitDestroyed";
const READ: u32 = Capability::Read as u32;
const SECRET: &str = "record-secret";
const DEADLINE: Duration = Duration::from_secs(60);

/// A record on [`TOPIC`] carrying `bytes` of string in field 1.
fn record(bytes: usize) -> Record {
    let mut e = Encoder::with_capacity(bytes + 128);
    e.begin(TOPIC.as_bytes(), None);
    e.string(1, &vec![b'x'; bytes]).unwrap();
    Arc::from(e.commit().unwrap())
}

/// The `dcsb` this build produced, with the token in its environment.
fn dcsb() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dcsb"));
    command.env("DCSB_TOKEN", SECRET);
    command
}

/// How many whole frames `path` holds, by walking the length prefixes.
/// `record` flushes each frame as it is written, so the count rises one
/// frame at a time.
fn frames_in(path: &PathBuf) -> usize {
    let bytes = fs::read(path).unwrap_or_default();
    let mut at = 0;
    let mut frames = 0;
    while at + 4 <= bytes.len() {
        let length = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        at += 4 + length as usize;
        if at > bytes.len() {
            break;
        }
        frames += 1;
    }
    frames
}

/// Wait for `child` to exit, or fail after the deadline.
fn wait(child: &mut Child, name: &str) -> std::process::ExitStatus {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "{name} did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Everything `child` wrote to the stream, read to its end on a thread so
/// a full pipe never blocks the child.
fn slurp(stream: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut text = String::new();
        BufReader::new(stream).read_to_string(&mut text).unwrap();
        text
    })
}

#[test]
fn record_replay_and_tail_round_trip_a_capture_as_processes() {
    const SIZES: [usize; 4] = [16, 300, 5000, 20000];
    let dir = std::env::temp_dir().join(format!("dcsb-replay-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let capture = dir.join("capture.bin");

    let (writer, mut commit, connections) = Writer::spawn(4096);
    let answers = Arc::new(dcsbridge_broker::state::Global);
    let listener =
        Listener::spawn("127.0.0.1:0", connections, Capacities::each(64), answers).unwrap();
    dcsbridge_broker::bridge().set_tokens(vec![Token {
        id: "record".into(),
        secret: SECRET.as_bytes().to_vec(),
        caps: [Capability::Read].into_iter().collect(),
    }]);

    let mut recorder = dcsb()
        .arg("record")
        .arg("--addr")
        .arg(listener.local_addr().to_string())
        .arg(&capture)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let recorder_err = slurp(recorder.stderr.take().unwrap());

    // A record committed before the writer thread knows the connection has
    // authenticated is passed over; a third frame in the file says it has.
    let deadline = Instant::now() + DEADLINE;
    while frames_in(&capture) < 3 {
        assert!(Instant::now() < deadline, "no record reached the file");
        drop(commit.push(READ, Class::Durable, record(1)));
        thread::sleep(Duration::from_millis(5));
    }
    let after_fillers = frames_in(&capture);
    for bytes in SIZES {
        drop(commit.push(READ, Class::Durable, record(bytes)));
    }
    // Fan-out is in order, so the last record in the file means every
    // record before it is there too.
    let deadline = Instant::now() + DEADLINE;
    while frames_in(&capture) < after_fillers + SIZES.len() {
        assert!(
            Instant::now() < deadline,
            "the sized records did not all reach the file"
        );
        thread::sleep(Duration::from_millis(5));
    }

    // Dropping the listener closes its connections between frames, which
    // `record` reads as the bridge going away.
    drop(listener);
    drop(writer);
    let status = wait(&mut recorder, "record");
    let recorder_err = recorder_err.join().unwrap();
    assert_eq!(status.code(), Some(0), "record: {recorder_err}");
    let frames = frames_in(&capture);
    assert!(
        recorder_err.contains(&format!("recorded {frames} frames, ")),
        "record did not report the file's frame count: {recorder_err}"
    );

    let mut replayer = dcsb()
        .arg("replay")
        .arg("--addr")
        .arg("127.0.0.1:0")
        .arg(&capture)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let replayer_err = slurp(replayer.stderr.take().unwrap());
    let mut replayer_out = BufReader::new(replayer.stdout.take().unwrap());
    let mut line = String::new();
    replayer_out.read_line(&mut line).unwrap();
    let addr = line
        .strip_prefix("listening on ")
        .unwrap_or_else(|| panic!("no listening line: {line:?}"))
        .trim()
        .to_owned();

    let mut tailer = dcsb()
        .arg("tail")
        .arg("--addr")
        .arg(&addr)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let tailer_out = slurp(tailer.stdout.take().unwrap());
    let tailer_err = slurp(tailer.stderr.take().unwrap());

    let replay_status = wait(&mut replayer, "replay");
    let tail_status = wait(&mut tailer, "tail");
    let replayer_err = replayer_err.join().unwrap();
    let tailer_out = tailer_out.join().unwrap();
    let tailer_err = tailer_err.join().unwrap();
    fs::remove_dir_all(&dir).unwrap();

    assert_eq!(replay_status.code(), Some(0), "replay: {replayer_err}");
    assert_eq!(
        tail_status.code(),
        Some(0),
        "tail: {tailer_err}\n{tailer_out}"
    );
    assert!(
        replayer_err.contains(&format!("replayed {frames} frames, ")),
        "replay did not report the file's frame count: {replayer_err}"
    );
    let seq_lines = tailer_out
        .lines()
        .filter(|line| line.starts_with("seq="))
        .count();
    assert_eq!(seq_lines, frames, "{tailer_out}");
    assert!(!tailer_out.contains("gap: "), "{tailer_out}");
    assert!(!tailer_out.contains("out of order"), "{tailer_out}");
    assert!(
        tailer_out.starts_with("seq=1 topic=dcsbridge.broker.Handshake "),
        "{tailer_out}"
    );
    assert!(
        tailer_err.contains(&format!(
            "connection closed after {frames} frames, 0 dropped in 0 gaps"
        )),
        "{tailer_err}"
    );
}

/// `record` refuses a file that ends inside a frame before it connects to
/// anything, naming where the last whole frame ends, and leaves the file as
/// it found it.
#[test]
fn record_refuses_a_file_that_ends_inside_a_frame() {
    let dir = std::env::temp_dir().join(format!("dcsb-partial-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let capture = dir.join("capture.bin");
    // One whole frame of four zero bytes, an empty envelope, then a frame
    // whose length prefix promises more than follows.
    let mut bytes = 0u32.to_le_bytes().to_vec();
    bytes.extend(64u32.to_le_bytes());
    bytes.extend([1, 2, 3]);
    fs::write(&capture, &bytes).unwrap();

    let output = dcsb()
        .arg("record")
        .arg("--addr")
        .arg("127.0.0.1:1")
        .arg(&capture)
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let left = fs::read(&capture).unwrap();
    fs::remove_dir_all(&dir).unwrap();

    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("ends inside a frame, after 4 whole bytes"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("cannot connect"),
        "record connected first: {stderr}"
    );
    assert_eq!(left, bytes, "the file was changed");
}
