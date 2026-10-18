//! What `protoc-gen-dcsbridge-lua` generates, apart from the stdin and stdout
//! it is driven through, so a test can hand it a request directly.
//!
//! A protoc plugin answers a `CodeGeneratorRequest` with a
//! `CodeGeneratorResponse`. A fault in the schema it was handed goes into the
//! response's `error`, which protoc or buf prints naming the plugin; the
//! plugin itself still exits zero.

use std::fmt::{self, Write};
use std::fs;

use prost::Message;
use prost_types::compiler::code_generator_response::{Feature, File};
use prost_types::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};
use sha2::{Digest, Sha256};

use emit::Side;

pub mod emit;
pub mod records;

/// Why generation refused the request, worded for the person who wrote the
/// schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub(crate) String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Answers the encoded request the plugin read from stdin.
///
/// The schema uses proto3 `optional` wherever absent must differ from zero,
/// and protoc refuses to hand such a file to a plugin that does not set this
/// feature bit, so the bit is set on every response, a refusal included.
pub fn respond(request: &[u8]) -> CodeGeneratorResponse {
    let supported_features = Some(Feature::Proto3Optional as u64);
    match generate(request) {
        Ok(file) => CodeGeneratorResponse {
            supported_features,
            file,
            ..Default::default()
        },
        Err(err) => CodeGeneratorResponse {
            error: Some(err.to_string()),
            supported_features,
            ..Default::default()
        },
    }
}

fn generate(request: &[u8]) -> Result<Vec<File>, Error> {
    let decoded = CodeGeneratorRequest::decode(request)
        .map_err(|err| Error(format!("the plugin request does not decode: {err}")))?;
    let schema_sha256 = schema_sha256(decoded.parameter.as_deref().unwrap_or_default())?;
    let records = records::read(request, &decoded)?.records;
    Ok([(emit::SIM_FILE, Side::Sim), (emit::HOOK_FILE, Side::Hook)]
        .into_iter()
        .map(|(name, side)| File {
            name: Some(name.to_owned()),
            content: Some(emit::file(side, &records, &schema_sha256)),
            ..Default::default()
        })
        .collect())
}

/// The plugin parameter's key naming the compiled schema.
const SCHEMA: &str = "schema";

/// The SHA-256 of the `schema.pb` the parameter names, as lowercase hex.
///
/// The broker hashes the bytes `shim.schema` hands it, which are that file,
/// so hashing the file itself is what makes the two equal. ADR 0039.
fn schema_sha256(parameter: &str) -> Result<String, Error> {
    let path = parameter
        .split(',')
        .find_map(|pair| pair.strip_prefix(SCHEMA)?.strip_prefix('='))
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            Error(format!(
                "the plugin parameter names no {SCHEMA}=<path to schema.pb>; \
                 the generated files carry that file's hash"
            ))
        })?;
    let bytes = fs::read(path).map_err(|err| {
        Error(format!(
            "the plugin parameter {SCHEMA}={path} does not read: {err}"
        ))
    })?;
    Ok(Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        }))
}

/// Builds plugin requests from the fixture schemas under `tests/proto`, the
/// way protoc and buf build them: every file the named ones import is in
/// `proto_file`, and `file_to_generate` names the fixtures alone.
#[cfg(test)]
pub(crate) mod testing {
    use prost::Message;

    /// A `CodeGeneratorRequest`, spelled so that each file stays the bytes
    /// protox wrote, extension options included.
    #[derive(Clone, PartialEq, Message)]
    struct Request {
        #[prost(string, repeated, tag = "1")]
        file_to_generate: Vec<String>,
        #[prost(string, optional, tag = "2")]
        parameter: Option<String>,
        #[prost(bytes = "vec", repeated, tag = "15")]
        proto_file: Vec<Vec<u8>>,
    }

    /// A `FileDescriptorSet`, with the same files held as bytes.
    #[derive(Clone, PartialEq, Message)]
    struct Set {
        #[prost(bytes = "vec", repeated, tag = "1")]
        file: Vec<Vec<u8>>,
    }

    pub(crate) fn request(files: &[&str], parameter: Option<&str>) -> Vec<u8> {
        let root = env!("CARGO_MANIFEST_DIR");
        let mut compiler =
            protox::Compiler::new([format!("{root}/tests/proto"), format!("{root}/../../proto")])
                .expect("the include paths resolve");
        compiler.include_imports(true);
        compiler.open_files(files).expect("the fixtures compile");
        let set = Set::decode(compiler.encode_file_descriptor_set().as_slice())
            .expect("protox wrote a descriptor set");
        Request {
            file_to_generate: files.iter().map(|f| (*f).to_owned()).collect(),
            parameter: parameter.map(str::to_owned),
            proto_file: set.file,
        }
        .encode_to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::request;

    /// `tests/lua/schema.lua` checks the broker's hash of the same three
    /// bytes against this constant.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn abc() -> String {
        format!("{SCHEMA}={}/tests/abc.pb", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn a_request_emits_both_files_carrying_the_hash_of_the_named_schema() {
        let response = respond(&request(&["fixture/records.proto"], Some(&abc())));
        assert_eq!(response.error, None);
        assert_eq!(
            response.supported_features,
            Some(Feature::Proto3Optional as u64)
        );
        let names: Vec<_> = response.file.iter().map(|f| f.name.as_deref()).collect();
        assert_eq!(names, [Some(emit::SIM_FILE), Some(emit::HOOK_FILE)]);
        for file in &response.file {
            let content = file.content.as_deref().unwrap_or_default();
            assert!(
                content.contains(&format!(".schema_sha256 = '{ABC_SHA256}'")),
                "{content}"
            );
        }
    }

    #[test]
    fn the_schema_parameter_may_sit_among_others() {
        let parameter = format!("other=1,{}", abc());
        let response = respond(&request(&["fixture/records.proto"], Some(&parameter)));
        assert_eq!(response.error, None);
    }

    #[test]
    fn a_request_without_the_schema_parameter_fails_naming_it() {
        let response = respond(&CodeGeneratorRequest::default().encode_to_vec());
        let error = response.error.expect("no error without the parameter");
        assert!(error.contains("schema=<path to schema.pb>"), "{error}");
        assert_eq!(
            response.supported_features,
            Some(Feature::Proto3Optional as u64)
        );
        assert!(response.file.is_empty());
    }

    #[test]
    fn a_schema_that_does_not_read_fails_naming_the_path() {
        let parameter = format!("{SCHEMA}=/nonexistent/schema.pb");
        let response = respond(&request(&["fixture/records.proto"], Some(&parameter)));
        let error = response.error.expect("no error for a missing file");
        assert!(error.contains("schema=/nonexistent/schema.pb"), "{error}");
    }

    #[test]
    fn a_request_that_does_not_decode_is_refused_in_the_response() {
        // Field 15 as a length-delimited value running past the end.
        let response = respond(&[0x7a, 0x05, 0x00]);
        let error = response.error.expect("no error for a truncated request");
        assert!(error.contains("does not decode"), "{error}");
        assert_eq!(
            response.supported_features,
            Some(Feature::Proto3Optional as u64)
        );
        assert!(response.file.is_empty());
    }
}
