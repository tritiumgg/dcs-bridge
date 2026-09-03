//! `dcsb`, the DCS-Bridge CLI.
//!
//! Nine verbs are planned and they arrive one at a time, each with the broker
//! behaviour it is there to observe. `tail` is the first: it connects to a
//! running bridge and prints each frame as it arrives, with a line wherever
//! the sequence numbers show that records were dropped.

mod tail;

use std::io::{self, BufReader, Write};
use std::net::TcpStream;
use std::process::ExitCode;

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
}

fn main() -> ExitCode {
    match Cli::parse().verb {
        Verb::Tail(args) => tail_verb(&args),
    }
}

/// Connect, then print frames until the bridge closes the connection.
///
/// A refused connection exits 2, because there is nothing to observe. A
/// stream that ends mid-frame or carries bytes no envelope decodes from exits
/// 1, after everything readable before it has been printed.
fn tail_verb(args: &TailArgs) -> ExitCode {
    let stream = match TcpStream::connect(&args.addr) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("cannot connect to {}: {error}", args.addr);
            return ExitCode::from(2);
        }
    };

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
