//! `dcsb`, the DCS-Bridge CLI.
//!
//! Eight verbs are planned and they arrive one at a time, each with the broker
//! behaviour it is there to observe. `tail` connects to a running bridge and
//! prints each frame as it arrives, with a line wherever the sequence
//! numbers show that records were dropped. `ping` asks whether the sim is
//! alive and exits by the answer, so a script can ask too. `schema` fetches
//! the set the bridge serves and writes it to a file, so it can be checked
//! against the deployed one. `send` puts one record on a topic, so a
//! command can be seen to reach the Lua state its route names. `record`
//! appends every frame a bridge sends to a file, as the wire carried it, so
//! a session can be kept and read back with no DCS running. `replay`
//! serves a capture to one consumer, so that consumer can be run against
//! the session with no DCS running. `doctor` checks an install from
//! outside DCS and prints one line per check, so a broken one is diagnosed
//! in one command.

mod dcslog;
mod doctor;
#[cfg(test)]
mod loopback;
mod ping;
mod record;
mod replay;
mod schema;
mod send;
mod tail;
mod wire;

use std::io::{self, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use std::{env, fs};

use clap::{Args, Parser, Subcommand};

/// Observe a running bridge and diagnose a broken one.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Print each frame a bridge sends, and each gap in its numbering.
    Tail(TailArgs),
    /// Ask a bridge whether the sim is alive, and exit 1 when it is not.
    Ping(PingArgs),
    /// Fetch the schema a bridge serves and write it to a file.
    Schema(SchemaArgs),
    /// Send one record on a topic, and exit 1 when the token is refused.
    Send(SendArgs),
    /// Append every frame a bridge sends to a file, until it closes.
    Record(RecordArgs),
    /// Serve a capture to the first consumer that connects, then exit.
    Replay(ReplayArgs),
    /// Check an install, one line per check, and exit 1 when one fails.
    Doctor(DoctorArgs),
}

/// The address the bridge listens on.
#[derive(Args)]
struct Addr {
    /// The address the bridge listens on.
    ///
    /// The default is the module's, until its first `configure` can move it.
    #[arg(long, default_value = "127.0.0.1:7742")]
    addr: String,
}

/// The address `replay` listens on.
#[derive(Args)]
struct Listen {
    /// The address to listen on for the one consumer.
    ///
    /// The default is beside the bridge's, so a bridge and a replay can run
    /// on one machine. Port 0 takes a free one, which the `listening on`
    /// line names.
    #[arg(long, default_value = "127.0.0.1:7743")]
    addr: String,
}

/// Where the token's secret comes from.
#[derive(Args)]
struct TokenFile {
    /// A file holding the token's secret, on its first line.
    ///
    /// Without it the secret is read from `DCSB_TOKEN`. A secret is never
    /// taken from the command line, where every process on the machine can
    /// read it.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,
}

#[derive(Args)]
struct TailArgs {
    #[command(flatten)]
    addr: Addr,

    #[command(flatten)]
    token: TokenFile,
}

#[derive(Args)]
struct PingArgs {
    #[command(flatten)]
    addr: Addr,
}

#[derive(Args)]
struct SchemaArgs {
    /// The file to write the schema to, replaced if it exists.
    #[arg(value_name = "PATH")]
    out: PathBuf,

    #[command(flatten)]
    addr: Addr,

    #[command(flatten)]
    token: TokenFile,
}

#[derive(Args)]
struct SendArgs {
    /// The topic: the record's fully qualified message name, such as
    /// `dcsbridge.builtin.sim.SetFlag`.
    #[arg(value_name = "TOPIC")]
    topic: String,

    /// A file holding the record's encoded bytes, sent whole.
    ///
    /// Without it and without `--hex`, the record is sent with no fields.
    #[arg(long, value_name = "PATH", conflicts_with = "hex")]
    file: Option<PathBuf>,

    /// The record's encoded bytes as hex, two digits per byte, spaces
    /// between bytes allowed.
    #[arg(long, value_name = "HEX")]
    hex: Option<String>,

    /// Seconds to keep reading after the record is sent, printing each
    /// frame that comes back as `tail` does. Nothing answers a record
    /// unless something sends an answer, so the default is not to wait.
    #[arg(long, value_name = "SECS", default_value_t = 0)]
    wait: u64,

    #[command(flatten)]
    addr: Addr,

    #[command(flatten)]
    token: TokenFile,
}

#[derive(Args)]
struct RecordArgs {
    /// The file to append the capture to, created if it does not exist.
    ///
    /// Each frame is written as the wire carried it, handshake first, so
    /// the file is what `replay` serves and what `tail` would have read.
    #[arg(value_name = "PATH")]
    out: PathBuf,

    #[command(flatten)]
    addr: Addr,

    #[command(flatten)]
    token: TokenFile,
}

#[derive(Args)]
struct ReplayArgs {
    /// The capture to serve, as `record` wrote it.
    #[arg(value_name = "PATH")]
    capture: PathBuf,

    #[command(flatten)]
    listen: Listen,
}

#[derive(Args)]
struct DoctorArgs {
    /// The Saved Games directory the release was extracted over.
    ///
    /// Without it, `Saved Games\DCS` and `Saved Games\DCS.openbeta` under
    /// `USERPROFILE` are checked, each that holds the loader.
    #[arg(long, value_name = "PATH")]
    dir: Option<PathBuf>,

    #[command(flatten)]
    addr: Addr,
}

/// The environment variable a secret is read from when no file names one.
const TOKEN_ENV: &str = "DCSB_TOKEN";

/// How long `ping` and `schema` wait for the answer. Loopback answers in
/// microseconds and the answer waits for nothing inside the bridge, so a
/// wait this long is a bridge that is not answering, and a script is told
/// so rather than held.
const ANSWER_WAIT: Duration = Duration::from_secs(5);

fn main() -> ExitCode {
    match Cli::parse().verb {
        Verb::Tail(args) => tail_verb(&args),
        Verb::Ping(args) => ping_verb(&args),
        Verb::Schema(args) => schema_verb(&args),
        Verb::Send(args) => send_verb(&args),
        Verb::Record(args) => record_verb(&args),
        Verb::Replay(args) => replay_verb(&args),
        Verb::Doctor(args) => doctor_verb(&args),
    }
}

/// Check each write directory found, then ask the port whether a bridge
/// answers, print one line per check, and exit by the lines.
///
/// Every check passed exits 0, with any notes printed. A check that failed
/// exits 1, after every other check has run. No write directory to check
/// exits 2, because nothing was learned. ADR 0075.
fn doctor_verb(args: &DoctorArgs) -> ExitCode {
    let home = env::var_os("USERPROFILE")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    let dirs = match doctor::write_dirs(args.dir.as_deref(), home.as_deref()) {
        Ok(dirs) => dirs,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut failed = false;
    for dir in &dirs {
        let report = doctor::check(dir);
        failed |= report.failed();
        let _ = write!(out, "{report}");
    }
    let _ = out.flush();
    let mut report = doctor::Report::default();
    doctor::port(&args.addr.addr, &mut report);
    failed |= report.failed();
    let _ = write!(out, "{report}");
    let _ = out.flush();
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Listen, accept one consumer, serve it the capture, and exit.
///
/// The capture served whole exits 0, after one line naming the frames and
/// bytes. A capture that ends inside a frame, or a consumer that leaves
/// before the end, exits 1, after every whole frame before it was sent. A
/// capture that cannot be opened or an address that cannot be bound exits
/// 2, because nothing was served. The `listening on` line goes to stdout
/// once the address is bound and before the accept, so a script that asked
/// for port 0 can read which port it got.
fn replay_verb(args: &ReplayArgs) -> ExitCode {
    let path = &args.capture;
    let capture = match fs::File::open(path) {
        Ok(file) => BufReader::with_capacity(1 << 16, file),
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            return ExitCode::from(2);
        }
    };
    let addr = &args.listen.addr;
    let listener = match TcpListener::bind(addr) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("cannot listen on {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    match listener.local_addr() {
        Ok(bound) => println!("listening on {bound}"),
        Err(error) => {
            eprintln!("cannot listen on {addr}: {error}");
            return ExitCode::from(2);
        }
    }
    let _ = io::stdout().flush();

    match replay::run(&listener, capture) {
        Ok(summary) => {
            eprintln!(
                "replayed {} frames, {} bytes from {}",
                summary.frames,
                summary.bytes,
                path.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("replay: {error}");
            ExitCode::from(1)
        }
    }
}

/// Connect, authenticate, then append frames to the file until the bridge
/// closes the connection.
///
/// A refused connection, no token to send, a token that could not be sent,
/// or a file that cannot be opened exits 2, because there is nothing to
/// record. So does a file that ends inside a frame, because a capture
/// appended after a partial frame reads as garbage from the join on. A
/// token the bridge refuses exits 1, after its answer has been written. A
/// stream that ends mid-frame exits 1, after every whole frame before it
/// has been written.
fn record_verb(args: &RecordArgs) -> ExitCode {
    let secret = match token(args.token.token_file.as_deref()) {
        Ok(secret) => secret,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let path = &args.out;
    match fs::File::open(path).map(record::whole_frames) {
        Ok(Ok(Ok(_))) => {}
        Ok(Ok(Err(offset))) => {
            eprintln!(
                "{} ends inside a frame, after {offset} whole bytes; move it, or truncate it to {offset} bytes",
                path.display()
            );
            return ExitCode::from(2);
        }
        Ok(Err(error)) => {
            eprintln!("{} is not a capture: {error}", path.display());
            return ExitCode::from(2);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("cannot read {}: {error}", path.display());
            return ExitCode::from(2);
        }
    }
    let sink = match fs::OpenOptions::new().append(true).create(true).open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            return ExitCode::from(2);
        }
    };
    let addr = &args.addr.addr;
    let mut stream = match TcpStream::connect(addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = stream.write_all(&wire::auth_frame(&secret)) {
        eprintln!("cannot send the token to {addr}: {error}");
        return ExitCode::from(2);
    }
    // The socket is read through a buffer for the reason `tail` reads it
    // through one: a burst is thousands of frames and one system call per
    // frame is what the reader falls behind on.
    let reader = BufReader::with_capacity(1 << 16, stream);

    match record::run(reader, sink) {
        Ok(summary) if summary.refused => {
            eprintln!("the bridge refused the token; its answer is in the file");
            ExitCode::from(1)
        }
        Ok(summary) => {
            eprintln!(
                "recorded {} frames, {} bytes to {}",
                summary.frames,
                summary.bytes,
                path.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("record: {error}");
            ExitCode::from(1)
        }
    }
}

/// Connect, authenticate, send one record, and exit by the token's answer.
///
/// The token accepted exits 0, after one line naming the topic and the
/// size, and after whatever `--wait` printed. The token refused exits 1,
/// and so does a wait cut mid-frame or fed bytes no envelope decodes from,
/// after everything readable before it has been printed, as `tail` exits:
/// the record was sent and that line has been printed already. Nothing
/// learned exits 2: no token or no payload to send, a refused connection,
/// a bridge that closes without answering or does not answer in time.
fn send_verb(args: &SendArgs) -> ExitCode {
    let secret = match token(args.token.token_file.as_deref()) {
        Ok(secret) => secret,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let value = match send::payload(args.file.as_deref(), args.hex.as_deref()) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let addr = &args.addr.addr;
    let mut stream = match TcpStream::connect(addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    // The token and the record go out together, as `schema` sends its
    // request: the bridge reads frames in order, and a refused token closes
    // the connection before the record is read.
    let bytes = value.len();
    let mut request = wire::auth_frame(&secret);
    request.extend(send::record_frame(&args.topic, value));
    if let Err(error) = stream.write_all(&request) {
        eprintln!("cannot send the record to {addr}: {error}");
        return ExitCode::from(2);
    }
    let mut reader = wire::Deadline::new(stream, ANSWER_WAIT);

    match send::run(&mut reader) {
        Ok(send::Outcome::Sent) => {
            println!("sent topic={} bytes={bytes}", args.topic);
            if args.wait == 0 {
                return ExitCode::SUCCESS;
            }
            let reader = reader.again(Duration::from_secs(args.wait));
            let stdout = io::stdout();
            let mut out = stdout.lock();
            let result = send::wait(reader, &mut out);
            let _ = out.flush();
            match result {
                Ok(_) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("send: {error}");
                    ExitCode::from(1)
                }
            }
        }
        Ok(send::Outcome::Refused(error)) => {
            eprintln!(
                "the bridge refused the token: {}",
                wire::auth_error_name(error)
            );
            ExitCode::from(1)
        }
        Ok(send::Outcome::Closed) => {
            eprintln!("{addr} closed the connection without answering");
            ExitCode::from(2)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            eprintln!(
                "no answer from {addr} within {} seconds",
                ANSWER_WAIT.as_secs()
            );
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("send: {error}");
            ExitCode::from(2)
        }
    }
}

/// Connect, authenticate, send one `GetSchema`, and write the set.
///
/// The set written exits 0, after one line naming its hash, size and path.
/// An answer that is not the set exits 1: the bridge holds no schema yet,
/// the token was refused, or the set does not hash to what the handshake
/// said and is not written. Nothing learned exits 2, as `ping`, and so does
/// a file that could not be written, because the set is not on disk.
fn schema_verb(args: &SchemaArgs) -> ExitCode {
    let secret = match token(args.token.token_file.as_deref()) {
        Ok(secret) => secret,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let addr = &args.addr.addr;
    let mut stream = match TcpStream::connect(addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    // The token and the request go out together: the bridge reads frames
    // in order, so the request is answered after the token is, and a
    // refused token closes the connection before it is read.
    let mut request = wire::auth_frame(&secret);
    request.extend(schema::get_schema_frame());
    if let Err(error) = stream.write_all(&request) {
        eprintln!("cannot send the request to {addr}: {error}");
        return ExitCode::from(2);
    }
    let reader = wire::Deadline::new(stream, ANSWER_WAIT);

    match schema::run(reader) {
        Ok(schema::Outcome::Fetched { set, sha256 }) => {
            let path = &args.out;
            if let Err(error) = fs::write(path, &set) {
                eprintln!("cannot write {}: {error}", path.display());
                return ExitCode::from(2);
            }
            println!(
                "sha256={} bytes={} path={}",
                schema::hex(&sha256),
                set.len(),
                path.display()
            );
            ExitCode::SUCCESS
        }
        Ok(schema::Outcome::Mismatch) => {
            eprintln!("the schema does not hash to what the handshake said; nothing written");
            ExitCode::from(1)
        }
        Ok(schema::Outcome::NoSchema(error)) => {
            eprintln!("the bridge has no schema: {error}");
            ExitCode::from(1)
        }
        Ok(schema::Outcome::Refused(error)) => {
            eprintln!(
                "the bridge refused the token: {}",
                wire::auth_error_name(error)
            );
            ExitCode::from(1)
        }
        Ok(schema::Outcome::Closed) => {
            eprintln!("{addr} closed the connection without answering");
            ExitCode::from(2)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            eprintln!(
                "no answer from {addr} within {} seconds",
                ANSWER_WAIT.as_secs()
            );
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("schema: {error}");
            ExitCode::from(2)
        }
    }
}

/// Connect, send one `Ping`, print the `Pong`, and exit by it.
///
/// The sim alive exits 0. The sim not alive exits 1, after the line has
/// been printed, because a disabled bridge or a sim mid-load is an answer.
/// A refused connection, a bridge that closes without answering or does
/// not answer in time, or an answer that does not decode exits 2, because
/// nothing was learned.
fn ping_verb(args: &PingArgs) -> ExitCode {
    let addr = &args.addr.addr;
    let reader = match ping::ask(addr, ANSWER_WAIT) {
        Ok(reader) => reader,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match ping::run(reader, &mut out) {
        Ok(Some(pong)) if pong.dcs_alive => ExitCode::SUCCESS,
        Ok(Some(_)) => ExitCode::from(1),
        Ok(None) => {
            eprintln!("{addr} closed the connection without answering");
            ExitCode::from(2)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            eprintln!(
                "no answer from {addr} within {} seconds",
                ANSWER_WAIT.as_secs()
            );
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("ping: {error}");
            ExitCode::from(2)
        }
    }
}

/// The token's secret: the file's first line, or the environment's value.
fn token(token_file: Option<&Path>) -> Result<String, String> {
    let secret = match token_file {
        Some(path) => {
            let text = fs::read_to_string(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            // An editor or a shell on Windows writes a byte order mark and
            // ends a line with a carriage return, neither of which is part
            // of a secret; a line ending is whatever the file's is.
            text.strip_prefix('\u{feff}')
                .unwrap_or(&text)
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r')
                .to_owned()
        }
        None => env::var(TOKEN_ENV).map_err(|_| {
            format!("no token: set {TOKEN_ENV}, or pass --token-file with a file holding one")
        })?,
    };
    if secret.is_empty() {
        return Err("the token is empty".into());
    }
    Ok(secret)
}

/// Connect, authenticate, then print frames until the bridge closes the
/// connection.
///
/// A refused connection, no token to send, or a token that could not be
/// sent exits 2, because there is nothing to observe. A token the bridge
/// refuses exits 1, after its answer has been printed. A stream that ends
/// mid-frame or carries bytes no envelope decodes from exits 1, after
/// everything readable before it has been printed.
fn tail_verb(args: &TailArgs) -> ExitCode {
    let secret = match token(args.token.token_file.as_deref()) {
        Ok(secret) => secret,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let addr = &args.addr.addr;
    let mut stream = match TcpStream::connect(addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = stream.write_all(&wire::auth_frame(&secret)) {
        eprintln!("cannot send the token to {addr}: {error}");
        return ExitCode::from(2);
    }

    // A frame is a few dozen bytes and a burst is thousands of them, so the
    // socket is read through a buffer: one system call fills it with a few
    // hundred frames rather than two per frame. Without it the reader, not
    // the bridge, can be what a burst outruns.
    let reader = BufReader::with_capacity(1 << 16, stream);

    let stdout = io::stdout();
    let mut out = stdout.lock();
    let result = tail::run(reader, &mut out);
    let _ = out.flush();
    match result {
        Ok(summary) if summary.refused => {
            eprintln!("the bridge refused the token; its answer is the line above");
            ExitCode::from(1)
        }
        Ok(summary) => {
            eprintln!(
                "connection closed after {} frames, {} dropped in {} gaps",
                summary.frames, summary.dropped, summary.gaps
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("tail: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The secret is the file's first line with the marks an editor or a
    /// shell adds taken off: a byte order mark, a carriage return, a
    /// second line. An empty first line is no token.
    #[test]
    fn a_token_file_yields_its_first_line_without_editor_marks() {
        let dir = std::env::temp_dir().join(format!("dcsb-token-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let read = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap();
            token(Some(&path))
        };

        assert_eq!(read("plain", b"correct-horse\n").unwrap(), "correct-horse");
        assert_eq!(read("crlf", b"correct-horse\r\n").unwrap(), "correct-horse");
        assert_eq!(read("cr", b"correct-horse\r").unwrap(), "correct-horse");
        assert_eq!(
            read("bom", b"\xef\xbb\xbfcorrect-horse\r\n").unwrap(),
            "correct-horse"
        );
        assert_eq!(
            read("two", b"correct-horse\nsecond line\n").unwrap(),
            "correct-horse"
        );
        assert_eq!(read("empty", b"\n").unwrap_err(), "the token is empty");

        fs::remove_dir_all(&dir).unwrap();
    }
}
