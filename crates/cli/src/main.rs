//! `dcsb`, the DCS-Bridge CLI.
//!
//! Nine verbs are planned and they arrive one at a time, each with the broker
//! behaviour it is there to observe. `tail` connects to a running bridge and
//! prints each frame as it arrives, with a line wherever the sequence
//! numbers show that records were dropped. `ping` asks whether the sim is
//! alive and exits by the answer, so a script can ask too. `schema` fetches
//! the set the bridge serves and writes it to a file, so it can be checked
//! against the deployed one. `send` puts one record on a topic, so a
//! command can be seen to reach the Lua state its route names.

mod ping;
mod schema;
mod send;
mod tail;
mod wire;

use std::io::{self, BufReader, Write};
use std::net::TcpStream;
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
    let mut stream = match TcpStream::connect(addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = stream.write_all(&ping::ping_frame()) {
        eprintln!("cannot send a ping to {addr}: {error}");
        return ExitCode::from(2);
    }
    // The wait is wall-clock over the whole answer rather than one read:
    // the frame is complete or the time is up, whichever comes first.
    let reader = wire::Deadline::new(stream, ANSWER_WAIT);

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
