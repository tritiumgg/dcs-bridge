//! `dcsb`, the DCS-Bridge CLI.
//!
//! Nine verbs are planned and they arrive one at a time, each with the broker
//! behaviour it is there to observe. `tail` is the first: it connects to a
//! running bridge and prints each frame as it arrives, with a line wherever
//! the sequence numbers show that records were dropped.

mod tail;

use std::io::{self, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::ExitCode;
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
}

#[derive(Args)]
struct TailArgs {
    /// The address the bridge listens on.
    ///
    /// The default is the module's, until its first `configure` can move it.
    #[arg(long, default_value = "127.0.0.1:7742")]
    addr: String,

    /// A file holding the token's secret, on its first line.
    ///
    /// Without it the secret is read from `DCSB_TOKEN`. A secret is never
    /// taken from the command line, where every process on the machine can
    /// read it.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,
}

/// The environment variable a secret is read from when no file names one.
const TOKEN_ENV: &str = "DCSB_TOKEN";

fn main() -> ExitCode {
    match Cli::parse().verb {
        Verb::Tail(args) => tail_verb(&args),
    }
}

/// The token's secret: the file's first line, or the environment's value.
fn token(args: &TailArgs) -> Result<String, String> {
    let secret = match &args.token_file {
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
    let secret = match token(args) {
        Ok(secret) => secret,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let mut stream = match TcpStream::connect(&args.addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {}: {error}", args.addr);
            return ExitCode::from(2);
        }
    };
    if let Err(error) = stream.write_all(&tail::auth_frame(&secret)) {
        eprintln!("cannot send the token to {}: {error}", args.addr);
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
            token(&TailArgs {
                addr: String::new(),
                token_file: Some(path),
            })
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
