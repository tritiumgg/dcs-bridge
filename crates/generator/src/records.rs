//! The records a plugin request describes, read from the four message
//! options `dcsbridge/broker/broker.proto` declares.
//!
//! `prost-types` decodes a `FileDescriptorProto` without its extension
//! fields, so the options are read through `prost-reflect` from each file's
//! raw bytes instead. ADR 0036.
//!
//! A record is a top-level message, in a file the request names for
//! generation and outside the broker's own package, that carries any of the
//! four options. A message carrying none is a field type, which never crosses
//! into or out of Lua on its own. ADR 0037.

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
    /// The topic id of the record `reply_to` names, resolved against this
    /// record's package when the schema spells a bare name.
    pub reply_to: Option<String>,
}

/// The broker frames, answers or knows by name every message in its own
/// package, so none of them is generated. ADR 0037.
const BROKER_PACKAGE: &str = "dcsbridge.broker";

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
/// file order and then declaration order. `decoded` is `request` already
/// decoded through `prost-types`.
pub fn read(request: &[u8], decoded: &CodeGeneratorRequest) -> Result<Vec<Record>, Error> {
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
        if file.package_name() == BROKER_PACKAGE {
            continue;
        }
        for message in file.messages() {
            if let Some(record) = record(&message, &options)? {
                records.push(record);
            }
        }
    }
    distinct_names(&records)?;
    resolve_replies(&mut records)?;
    Ok(records)
}

/// The topics table is keyed by name, so two packages may not share one.
/// ADR 0021.
fn distinct_names(records: &[Record]) -> Result<(), Error> {
    for (index, record) in records.iter().enumerate() {
        if let Some(earlier) = records[..index].iter().find(|r| r.name == record.name) {
            return Err(Error(format!(
                "{} and {} share the name {}, which the topics table is keyed by",
                earlier.topic, record.topic, record.name
            )));
        }
    }
    Ok(())
}

/// Turns each `reply_to` into the topic id of the record it names, and fails
/// one that names anything else.
fn resolve_replies(records: &mut [Record]) -> Result<(), Error> {
    let topics: Vec<String> = records.iter().map(|r| r.topic.clone()).collect();
    for record in records.iter_mut() {
        let Some(spelled) = record.reply_to.take() else {
            continue;
        };
        let package = &record.topic[..record.topic.len() - record.name.len()];
        let topic = if spelled.contains('.') {
            spelled.clone()
        } else {
            format!("{package}{spelled}")
        };
        if !topics.contains(&topic) {
            return Err(Error(format!(
                "{} names {spelled} in ({REPLY_TO}), which is not a record \
                 in the files named for generation",
                record.topic
            )));
        }
        record.reply_to = Some(topic);
    }
    Ok(())
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

    fn read(request: &[u8]) -> Result<Vec<Record>, Error> {
        super::read(request, &CodeGeneratorRequest::decode(request).unwrap())
    }

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
                reply_to: Some("fixture.records.Spotted".into()),
            }
        );
    }

    #[test]
    fn the_imports_are_never_read() {
        // records.proto is in the request as an import, with two records of
        // its own, and only the file importing it is named for generation.
        let records = read(&request(&["fixture/imports_records.proto"], None)).unwrap();
        assert_eq!(topics(&records), ["fixture.imports_records.Seen"]);
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
    fn the_broker_package_is_never_generated() {
        // buf names broker.proto for generation, and its SetEnabled and
        // CommandAck each carry one option of the two a record needs.
        let records = read(&request(&["dcsbridge/broker/broker.proto"], None)).unwrap();
        assert!(records.is_empty(), "{records:?}");
    }

    #[test]
    fn two_packages_may_not_share_a_record_name() {
        let files = ["fixture/collision_a.proto", "fixture/collision_b.proto"];
        let err = read(&request(&files, None)).unwrap_err().to_string();
        assert!(err.contains("fixture.collision_a.Spotted"), "{err}");
        assert!(err.contains("fixture.collision_b.Spotted"), "{err}");
    }

    #[test]
    fn a_reply_resolves_by_bare_name_and_by_full_name() {
        let records = read(&request(&["fixture/reply.proto"], None)).unwrap();
        let replies: Vec<_> = records.iter().map(|r| r.reply_to.as_deref()).collect();
        assert_eq!(
            replies,
            [
                Some("fixture.reply.Position"),
                Some("fixture.reply.Position"),
                None
            ]
        );
    }

    #[test]
    fn a_reply_that_names_a_field_type_fails() {
        let err = read(&request(&["fixture/reply_to_field_type.proto"], None)).unwrap_err();
        let err = err.to_string();
        assert!(
            err.contains("fixture.reply_to_field_type.GetPosition"),
            "{err}"
        );
        assert!(err.contains("Point"), "{err}");
        assert!(err.contains("(dcsbridge.broker.reply_to)"), "{err}");
    }

    #[test]
    fn a_message_with_a_reply_alone_is_a_record_and_fails() {
        let err = read(&request(&["fixture/reply_to_only.proto"], None)).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("fixture.reply_to_only.GetPosition"), "{err}");
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
