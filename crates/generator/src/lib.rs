//! What `protoc-gen-dcsbridge-lua` generates, apart from the stdin and stdout
//! it is driven through, so a test can hand it a request directly.
//!
//! A protoc plugin answers a `CodeGeneratorRequest` with a
//! `CodeGeneratorResponse`. A fault in the schema it was handed goes into the
//! response's `error`, which protoc or buf prints naming the plugin; the
//! plugin itself still exits zero.

use std::fmt;

use prost::Message;
use prost_types::compiler::code_generator_response::{Feature, File};
use prost_types::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};

/// Why generation refused the request, worded for the person who wrote the
/// schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

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
    let _request = CodeGeneratorRequest::decode(request)
        .map_err(|err| Error(format!("the plugin request does not decode: {err}")))?;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_request_advertises_proto3_optional_and_emits_nothing() {
        let response = respond(&CodeGeneratorRequest::default().encode_to_vec());
        assert_eq!(response.error, None);
        assert_eq!(
            response.supported_features,
            Some(Feature::Proto3Optional as u64)
        );
        assert!(response.file.is_empty());
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
