//! `protoc-gen-dcsbridge-lua`, the DCS-Bridge protoc plugin.
//!
//! protoc or buf writes a `CodeGeneratorRequest` to stdin and reads a
//! `CodeGeneratorResponse` from stdout. A fault in the schema travels inside
//! the response; only a failure to read or write the pipe exits non-zero.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use prost::Message;

fn main() -> ExitCode {
    let mut request = Vec::new();
    if let Err(err) = io::stdin().read_to_end(&mut request) {
        eprintln!("protoc-gen-dcsbridge-lua: reading the request: {err}");
        return ExitCode::FAILURE;
    }
    let response = protoc_gen_dcsbridge_lua::respond(&request).encode_to_vec();
    let mut stdout = io::stdout().lock();
    if let Err(err) = stdout.write_all(&response).and_then(|()| stdout.flush()) {
        eprintln!("protoc-gen-dcsbridge-lua: writing the response: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
