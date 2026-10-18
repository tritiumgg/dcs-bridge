//! The records a plugin request describes, read from the four message
//! options `dcsbridge/broker/broker.proto` declares.
//!
//! `prost-types` decodes a `FileDescriptorProto` without its extension
//! fields, so the options are read through `prost-reflect` from each file's
//! raw bytes instead. ADR 0036.
//!
//! A record is a top-level message, in a file the request names for
//! generation, that carries any of the four options. A message carrying none
//! is a field type, which never crosses into or out of Lua on its own.
//! ADR 0037.

use prost::Message;
use prost_reflect::{DescriptorPool, ExtensionDescriptor, MessageDescriptor};
use prost_types::compiler::CodeGeneratorRequest;

use crate::Error;

/// One record: the topic it travels on and the four options it carries.
///
/// The enum values are the schema's numbers, unspecified as zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The topic id, which is the message's fully-qualified name.
    pub topic: String,
    /// The message's name without its package.
    pub name: String,
    /// `RecordClass`, never zero.
    pub class: i32,
    /// `Target`, where zero reads as the sim driver.
    pub target: i32,
    /// `Capability`, never zero.
    pub capability: i32,
    /// The `reply_to` option as the schema spells it.
    pub reply_to: Option<String>,
}

/// The extension names, spelled as `broker.proto` declares them.
const RECORD_CLASS: &str = "dcsbridge.broker.record_class";
const REPLY_TO: &str = "dcsbridge.broker.reply_to";
const TARGET: &str = "dcsbridge.broker.target";
const REQUIRED_CAPABILITY: &str = "dcsbridge.broker.required_capability";

/// The request's `proto_file` field, kept as bytes so that the extension
/// options inside each file survive to `prost-reflect`.
#[derive(Clone, PartialEq, Message)]
struct RawProtoFiles {
    #[prost(bytes = "vec", repeated, tag = "15")]
    proto_file: Vec<Vec<u8>>,
}

struct Options {
    record_class: ExtensionDescriptor,
    reply_to: ExtensionDescriptor,
    target: ExtensionDescriptor,
    required_capability: ExtensionDescriptor,
}

/// Reads every record in the files the request names for generation, in
/// file order and then declaration order.
pub fn read(request: &[u8]) -> Result<Vec<Record>, Error> {
    let decoded = CodeGeneratorRequest::decode(request)
        .map_err(|err| Error(format!("the plugin request does not decode: {err}")))?;
    let raw = RawProtoFiles::decode(request)
        .map_err(|err| Error(format!("the plugin request does not decode: {err}")))?;

    // protoc and buf list every file after the files it imports, so each one
    // resolves against the pool built so far.
    let mut pool = DescriptorPool::new();
    for file in &raw.proto_file {
        pool.decode_file_descriptor_proto(file.as_slice())
            .map_err(|err| Error(format!("a file in the plugin request is invalid: {err}")))?;
    }

    // A schema that never imports broker.proto declares no option, so no
    // message in it is a record.
    let Some(options) = extensions(&pool) else {
        return Ok(Vec::new());
    };

    let mut records = Vec::new();
    for name in &decoded.file_to_generate {
        let file = pool.get_file_by_name(name).ok_or_else(|| {
            Error(format!(
                "{name} is named for generation and is not in the request"
            ))
        })?;
        for message in file.messages() {
            if let Some(record) = record(&message, &options)? {
                records.push(record);
            }
        }
    }
    Ok(records)
}

fn extensions(pool: &DescriptorPool) -> Option<Options> {
    Some(Options {
        record_class: pool.get_extension_by_name(RECORD_CLASS)?,
        reply_to: pool.get_extension_by_name(REPLY_TO)?,
        target: pool.get_extension_by_name(TARGET)?,
        required_capability: pool.get_extension_by_name(REQUIRED_CAPABILITY)?,
    })
}

fn record(message: &MessageDescriptor, options: &Options) -> Result<Option<Record>, Error> {
    let set = message.options();
    let all = [
        &options.record_class,
        &options.reply_to,
        &options.target,
        &options.required_capability,
    ];
    if !all.iter().any(|option| set.has_extension(option)) {
        return Ok(None);
    }

    let number =
        |option: &ExtensionDescriptor| set.get_extension(option).as_enum_number().unwrap_or(0);
    let class = number(&options.record_class);
    let capability = number(&options.required_capability);
    let topic = message.full_name();
    for (value, option) in [(class, RECORD_CLASS), (capability, REQUIRED_CAPABILITY)] {
        if value == 0 {
            return Err(Error(format!(
                "{topic} carries a record option and no ({option}): \
                 a record names a class and a capability"
            )));
        }
    }

    let reply_to = set.has_extension(&options.reply_to).then(|| {
        set.get_extension(&options.reply_to)
            .as_str()
            .unwrap_or_default()
            .to_owned()
    });
    Ok(Some(Record {
        topic: topic.to_owned(),
        name: message.name().to_owned(),
        class,
        target: number(&options.target),
        capability,
        reply_to,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::request;

    fn topics(records: &[Record]) -> Vec<&str> {
        records.iter().map(|r| r.topic.as_str()).collect()
    }

    #[test]
    fn a_message_with_no_option_is_a_field_type_and_nested_types_are_skipped() {
        let records = read(&request(&["fixture/records.proto"], None)).unwrap();
        assert_eq!(
            topics(&records),
            ["fixture.records.Spotted", "fixture.records.SetMark"]
        );
    }

    #[test]
    fn a_record_reads_its_four_options() {
        let records = read(&request(&["fixture/records.proto"], None)).unwrap();
        assert_eq!(
            records[0],
            Record {
                topic: "fixture.records.Spotted".into(),
                name: "Spotted".into(),
                class: 2,
                target: 0,
                capability: 1,
                reply_to: None,
            }
        );
        assert_eq!(
            records[1],
            Record {
                topic: "fixture.records.SetMark".into(),
                name: "SetMark".into(),
                class: 3,
                target: 2,
                capability: 2,
                reply_to: Some("Spotted".into()),
            }
        );
    }

    #[test]
    fn the_imports_are_never_read() {
        // broker.proto and descriptor.proto are in the request as imports,
        // and SetEnabled in broker.proto carries a capability and no class.
        let records = read(&request(&["fixture/records.proto"], None)).unwrap();
        assert!(records.iter().all(|r| r.topic.starts_with("fixture.")));
    }

    #[test]
    fn a_record_without_a_capability_fails_naming_the_message_and_the_option() {
        let err = read(&request(&["fixture/no_capability.proto"], None)).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("fixture.no_capability.Spotted"), "{err}");
        assert!(
            err.contains("(dcsbridge.broker.required_capability)"),
            "{err}"
        );
    }

    #[test]
    fn a_record_without_a_class_fails_naming_the_message_and_the_option() {
        let err = read(&request(&["fixture/no_class.proto"], None)).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("fixture.no_class.Spotted"), "{err}");
        assert!(err.contains("(dcsbridge.broker.record_class)"), "{err}");
    }

    #[test]
    fn a_message_with_a_target_alone_is_a_record_and_fails() {
        let err = read(&request(&["fixture/target_only.proto"], None)).unwrap_err();
        assert!(
            err.to_string().contains("fixture.target_only.Spotted"),
            "{err}"
        );
    }
}
