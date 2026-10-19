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
//!
//! Every field of a record, and of each field type a record reaches, crosses
//! as one of the shim's put calls. A field whose kind has no put fails
//! generation. ADR 0040.

use prost::Message;
use prost_reflect::{
    DescriptorPool, ExtensionDescriptor, FieldDescriptor, Kind as ProtoKind, MessageDescriptor,
};
use prost_types::compiler::CodeGeneratorRequest;

use crate::Error;

/// What a request describes: its records, and the field types they reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// The packages of the files named for generation, sorted, each once,
    /// whether or not they hold a record.
    pub packages: Vec<String>,
    /// In file order and then declaration order.
    pub records: Vec<Record>,
    /// The broker's acknowledgement, when the request holds `broker.proto`.
    /// The shipped files carry its emitter and its `send_to` wrapper.
    pub ack: Option<Record>,
    /// Every message the records' and the acknowledgement's fields reach,
    /// directly or through another field type, sorted by full name.
    pub types: Vec<Type>,
}

/// A message a record's fields reach, which is encoded inside its record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
    /// The message's fully-qualified name.
    pub name: String,
    /// In declaration order.
    pub fields: Vec<Field>,
}

/// One field of a record or of a field type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The schema's name, verbatim.
    pub name: String,
    /// The field number its put call names.
    pub number: u32,
    /// The put call it crosses through.
    pub kind: Kind,
    /// Whether it holds one value, an optional one or a list.
    pub label: Label,
}

/// A field's kind, as finely as a decoder tells kinds apart on the wire.
///
/// Only some kinds have a put call. An outbound record that reaches one
/// without fails generation; a command, which is decoded and never put, may
/// carry any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A varint whose low 32 bits are a signed value.
    Int32,
    /// A varint holding a signed 64-bit value.
    Int64,
    /// A varint whose low 32 bits are an unsigned value.
    Uint32,
    /// A varint holding an unsigned 64-bit value.
    Uint64,
    /// A varint read as `Int32` is.
    Enum,
    /// A zigzag varint, 32 bits.
    Sint32,
    /// A zigzag varint, 64 bits.
    Sint64,
    /// Four little-endian bytes, unsigned.
    Fixed32,
    /// Eight little-endian bytes, unsigned.
    Fixed64,
    /// Four little-endian bytes, signed.
    Sfixed32,
    /// Eight little-endian bytes, signed.
    Sfixed64,
    /// An IEEE 754 single.
    Float,
    /// An IEEE 754 double.
    Double,
    /// Strings and bytes, which Lua holds alike.
    String,
    /// A varint, zero false and any other value true.
    Boolean,
    /// A field type, by full name.
    Message(String),
    /// A map, by the full name of its entry message.
    Map(String),
    /// A proto2 group.
    Group,
}

impl Kind {
    /// The shim's put call for a scalar it writes: every integer kind with a
    /// plain varint, enums, doubles, strings and booleans. ADR 0040.
    pub fn put(&self) -> Option<&'static str> {
        match self {
            Self::Int32 | Self::Int64 | Self::Uint32 | Self::Uint64 | Self::Enum => Some("integer"),
            Self::Double => Some("double"),
            Self::String => Some("string"),
            Self::Boolean => Some("boolean"),
            _ => None,
        }
    }

    /// The schema's word for a kind the shim has no put for: it has no
    /// zigzag, fixed-width or float put, and no map or group.
    pub fn unwritable(&self) -> Option<&'static str> {
        match self {
            Self::Sint32 => Some("sint32"),
            Self::Sint64 => Some("sint64"),
            Self::Fixed32 => Some("fixed32"),
            Self::Fixed64 => Some("fixed64"),
            Self::Sfixed32 => Some("sfixed32"),
            Self::Sfixed64 => Some("sfixed64"),
            Self::Float => Some("float"),
            Self::Map(_) => Some("map"),
            Self::Group => Some("group"),
            _ => None,
        }
    }

    /// The full name of the message a field of this kind holds on the wire:
    /// its field type, or its map entry.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Message(name) | Self::Map(name) => Some(name),
            _ => None,
        }
    }
}

/// How many values a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// One value, absent reading as zero.
    Single,
    /// One value, absent distinct from zero: proto3 `optional`.
    Optional,
    /// Any number of values, in order.
    Repeated,
}

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
    /// In declaration order.
    pub fields: Vec<Field>,
}

impl Record {
    /// The package the message is declared in, empty for none.
    pub fn package(&self) -> &str {
        self.topic
            .strip_suffix(self.name.as_str())
            .and_then(|rest| rest.strip_suffix('.'))
            .unwrap_or_default()
    }
}

/// The broker frames, answers or knows by name every message in its own
/// package, so none of them is generated. ADR 0037.
const BROKER_PACKAGE: &str = "dcsbridge.broker";

/// The extension names, spelled as `broker.proto` declares them.
const RECORD_CLASS: &str = "dcsbridge.broker.record_class";
const REPLY_TO: &str = "dcsbridge.broker.reply_to";
const TARGET: &str = "dcsbridge.broker.target";
const REQUIRED_CAPABILITY: &str = "dcsbridge.broker.required_capability";

/// `RecordClass`'s command member, the class an inbound record carries.
const RECORD_CLASS_COMMAND: i32 = 3;

/// The acknowledgement's topic.
const ACKNOWLEDGEMENT: &str = "dcsbridge.broker.CommandAck";

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
/// file order and then declaration order, and the field types they reach.
/// `decoded` is `request` already decoded through `prost-types`.
pub fn read(request: &[u8], decoded: &CodeGeneratorRequest) -> Result<Schema, Error> {
    let raw = RawProtoFiles::decode(request)
        .map_err(|err| Error(format!("the plugin request does not decode: {err}")))?;

    // protoc and buf list every file after the files it imports, so each one
    // resolves against the pool built so far.
    let mut pool = DescriptorPool::new();
    for file in &raw.proto_file {
        pool.decode_file_descriptor_proto(file.as_slice())
            .map_err(|err| Error(format!("a file in the plugin request is invalid: {err}")))?;
    }

    let files = decoded
        .file_to_generate
        .iter()
        .map(|name| {
            pool.get_file_by_name(name).ok_or_else(|| {
                Error(format!(
                    "{name} is named for generation and is not in the request"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut packages: Vec<String> = files.iter().map(|f| f.package_name().to_owned()).collect();
    packages.sort_unstable();
    packages.dedup();

    // A schema that never imports broker.proto declares no option, so no
    // message in it is a record.
    let Some(options) = extensions(&pool) else {
        return Ok(Schema {
            packages,
            records: Vec::new(),
            ack: None,
            types: Vec::new(),
        });
    };

    let mut records = Vec::new();
    for file in files {
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
    let ack = acknowledgement(&pool, &options)?;
    let types = field_types(&pool, records.iter().chain(&ack))?;
    refuse_unwritable(&records, ack.as_ref(), &types)?;
    Ok(Schema {
        packages,
        records,
        ack,
        types,
    })
}

/// The acknowledgement, the one message of the broker's package a handler
/// sends. It is a reply with no table rows, since the broker knows it by
/// name, and it carries no capability. ADR 0040.
fn acknowledgement(pool: &DescriptorPool, options: &Options) -> Result<Option<Record>, Error> {
    let Some(message) = pool.get_message_by_name(ACKNOWLEDGEMENT) else {
        return Ok(None);
    };
    let class = message
        .options()
        .get_extension(&options.record_class)
        .as_enum_number()
        .unwrap_or(0);
    Ok(Some(Record {
        topic: ACKNOWLEDGEMENT.to_owned(),
        name: message.name().to_owned(),
        class,
        target: 0,
        capability: 0,
        reply_to: None,
        fields: fields(&message),
    }))
}

/// Every message the records' fields reach, followed through field types
/// until none is new.
fn field_types<'a>(
    pool: &DescriptorPool,
    records: impl IntoIterator<Item = &'a Record>,
) -> Result<Vec<Type>, Error> {
    let mut pending: Vec<String> = records
        .into_iter()
        .flat_map(|r| message_kinds(&r.fields))
        .collect();
    let mut types: Vec<Type> = Vec::new();
    while let Some(name) = pending.pop() {
        if types.iter().any(|t| t.name == name) {
            continue;
        }
        // A field's kind names a message the pool resolved, so it is there.
        let message = pool
            .get_message_by_name(&name)
            .ok_or_else(|| Error(format!("{name} is a field type and is not in the request")))?;
        let fields = fields(&message);
        pending.extend(message_kinds(&fields));
        types.push(Type { name, fields });
    }
    types.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(types)
}

/// The message each field holds on the wire: a field type, or a map's entry.
fn message_kinds(fields: &[Field]) -> impl Iterator<Item = String> + '_ {
    fields
        .iter()
        .filter_map(|f| f.kind.message().map(str::to_owned))
}

/// The longest chain of field types below any of `commands`, each link one
/// message nested in another. A type met again along one chain counts once
/// and ends it, so a type that contains itself adds two levels and no more.
/// A map's entry is a level. ADR 0045.
pub fn depth(commands: &[&Record], types: &[Type]) -> usize {
    fn below(fields: &[Field], types: &[Type], chain: &mut Vec<String>) -> usize {
        message_kinds(fields)
            .map(|name| {
                if chain.contains(&name) {
                    return 1;
                }
                let Some(found) = types.iter().find(|t| t.name == name) else {
                    return 1;
                };
                chain.push(name);
                let levels = 1 + below(&found.fields, types, chain);
                chain.pop();
                levels
            })
            .max()
            .unwrap_or(0)
    }
    commands
        .iter()
        .map(|r| below(&r.fields, types, &mut Vec::new()))
        .max()
        .unwrap_or(0)
}

/// The message's fields in declaration order, which is the order the
/// emitter writes them in. `MessageDescriptor::fields` sorts by number, so
/// the order comes from the descriptor proto instead.
fn fields(message: &MessageDescriptor) -> Vec<Field> {
    message
        .descriptor_proto()
        .field
        .iter()
        .filter_map(|proto| message.get_field_by_name(proto.name()))
        .map(|f| field(&f))
        .collect()
}

/// One field, with its kind and label.
fn field(field: &FieldDescriptor) -> Field {
    let kind = match field.kind() {
        ProtoKind::Message(entry) if field.is_map() => Kind::Map(entry.full_name().to_owned()),
        _ if field.is_group() => Kind::Group,
        other => kind(other),
    };
    let label = if field.is_list() {
        Label::Repeated
    } else if field.containing_oneof().is_some_and(|o| o.is_synthetic()) {
        Label::Optional
    } else {
        Label::Single
    };
    Field {
        name: field.name().to_owned(),
        number: field.number(),
        kind,
        label,
    }
}

fn kind(kind: ProtoKind) -> Kind {
    match kind {
        ProtoKind::Int32 => Kind::Int32,
        ProtoKind::Int64 => Kind::Int64,
        ProtoKind::Uint32 => Kind::Uint32,
        ProtoKind::Uint64 => Kind::Uint64,
        ProtoKind::Enum(_) => Kind::Enum,
        ProtoKind::Sint32 => Kind::Sint32,
        ProtoKind::Sint64 => Kind::Sint64,
        ProtoKind::Fixed32 => Kind::Fixed32,
        ProtoKind::Fixed64 => Kind::Fixed64,
        ProtoKind::Sfixed32 => Kind::Sfixed32,
        ProtoKind::Sfixed64 => Kind::Sfixed64,
        ProtoKind::Float => Kind::Float,
        ProtoKind::Double => Kind::Double,
        ProtoKind::String | ProtoKind::Bytes => Kind::String,
        ProtoKind::Bool => Kind::Boolean,
        ProtoKind::Message(of) => Kind::Message(of.full_name().to_owned()),
    }
}

/// Fails naming a field with no put call that an emitter would write:
/// one on an outbound record, on the acknowledgement, or on a field type
/// either reaches. A command is decoded and never put, so its fields are
/// not checked here. ADR 0040.
fn refuse_unwritable(
    records: &[Record],
    ack: Option<&Record>,
    types: &[Type],
) -> Result<(), Error> {
    let mut pending: Vec<(&str, &Field)> = records
        .iter()
        .filter(|r| r.class != RECORD_CLASS_COMMAND)
        .chain(ack)
        .flat_map(|r| r.fields.iter().map(move |f| (r.topic.as_str(), f)))
        .collect();
    let mut seen: Vec<&str> = Vec::new();
    while let Some((message, field)) = pending.pop() {
        if let Some(what) = field.kind.unwritable() {
            return Err(Error(format!(
                "{message}.{} is a {what} field, and the shim has no put call for one",
                field.name
            )));
        }
        match &field.kind {
            Kind::Message(name) if !seen.contains(&name.as_str()) => {
                seen.push(name);
                if let Some(found) = types.iter().find(|t| &t.name == name) {
                    pending.extend(found.fields.iter().map(|f| (found.name.as_str(), f)));
                }
            }
            _ => {}
        }
    }
    Ok(())
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
        fields: fields(message),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::request;

    fn schema(request: &[u8]) -> Result<Schema, Error> {
        super::read(request, &CodeGeneratorRequest::decode(request).unwrap())
    }

    fn read(request: &[u8]) -> Result<Vec<Record>, Error> {
        schema(request).map(|s| s.records)
    }

    fn field(name: &str, number: u32, kind: Kind, label: Label) -> Field {
        Field {
            name: name.into(),
            number,
            kind,
            label,
        }
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
                fields: vec![field(
                    "at",
                    1,
                    Kind::Message("fixture.records.Point".into()),
                    Label::Single
                )],
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
                fields: vec![field(
                    "label",
                    1,
                    Kind::Message("fixture.records.SetMark.Label".into()),
                    Label::Single
                )],
            }
        );
    }

    #[test]
    fn the_field_types_a_record_reaches_are_read_and_sorted() {
        let types = schema(&request(&["fixture/records.proto"], None))
            .unwrap()
            .types;
        assert_eq!(
            types,
            [
                Type {
                    name: "fixture.records.Point".into(),
                    fields: vec![
                        field("x", 1, Kind::Double, Label::Single),
                        field("z", 2, Kind::Double, Label::Single),
                    ],
                },
                Type {
                    name: "fixture.records.SetMark.Label".into(),
                    fields: vec![field("text", 1, Kind::String, Label::Single)],
                },
            ]
        );
    }

    #[test]
    fn each_kind_reads_as_itself_and_fields_keep_declaration_order() {
        let schema = schema(&request(&["fixture/kinds.proto"], None)).unwrap();
        let point = Kind::Message("fixture.kinds.Point".into());
        let fields = &schema.records[0].fields;
        assert_eq!(
            fields,
            &[
                field("count", 3, Kind::Int32, Label::Single),
                field("total", 1, Kind::Int64, Label::Single),
                field("seen", 2, Kind::Uint32, Label::Single),
                field("size", 4, Kind::Uint64, Label::Single),
                field("side", 5, Kind::Enum, Label::Single),
                field("hit", 6, Kind::Boolean, Label::Single),
                field("speed", 7, Kind::Double, Label::Single),
                field("name", 8, Kind::String, Label::Single),
                field("blob", 9, Kind::String, Label::Single),
                field("note", 10, Kind::String, Label::Optional),
                field("ids", 11, Kind::Int32, Label::Repeated),
                field("path", 12, point.clone(), Label::Repeated),
                field("at", 13, point, Label::Single),
            ]
        );
        let puts: Vec<_> = fields[..9].iter().map(|f| f.kind.put()).collect();
        assert_eq!(
            puts,
            [
                "integer", "integer", "integer", "integer", "integer", "boolean", "double",
                "string", "string",
            ]
            .map(Some)
        );
        // Point reaches Grid, which a record names nowhere.
        let names: Vec<_> = schema.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["fixture.kinds.Grid", "fixture.kinds.Point"]);
    }

    #[test]
    fn a_kind_with_no_put_fails_naming_the_message_and_the_field() {
        for (file, at, kind) in [
            (
                "fixture/sint_field.proto",
                "fixture.sint_field.Spotted.offset",
                "sint32",
            ),
            (
                "fixture/float_field.proto",
                "fixture.float_field.Point.x",
                "float",
            ),
            (
                "fixture/map_field.proto",
                "fixture.map_field.Spotted.tags",
                "map",
            ),
        ] {
            let err = read(&request(&[file], None)).unwrap_err().to_string();
            assert!(err.contains(at), "{err}");
            assert!(err.contains(&format!("is a {kind} field")), "{err}");
        }
    }

    #[test]
    fn a_command_may_carry_every_kind_with_no_put_and_each_reads_as_its_name() {
        let schema = schema(&request(&["fixture/command_kinds.proto"], None)).unwrap();
        let kinds: Vec<_> = schema.records[0]
            .fields
            .iter()
            .map(|f| f.kind.clone())
            .collect();
        assert_eq!(
            kinds[..9],
            [
                Kind::Sint32,
                Kind::Sint64,
                Kind::Float,
                Kind::Fixed32,
                Kind::Fixed64,
                Kind::Sfixed32,
                Kind::Sfixed64,
                Kind::Map("fixture.command_kinds.SetAltitude.HEntry".into()),
                Kind::Group,
            ]
        );
        let words: Vec<_> = kinds[..9].iter().map(|k| k.unwritable()).collect();
        assert_eq!(
            words,
            [
                "sint32", "sint64", "float", "fixed32", "fixed64", "sfixed32", "sfixed64", "map",
                "group",
            ]
            .map(Some)
        );
        // The field type only the command reaches is read, and not refused.
        let point = schema
            .types
            .iter()
            .find(|t| t.name == "fixture.command_kinds.Point");
        assert_eq!(point.unwrap().fields[0].kind, Kind::Float);
    }

    #[test]
    fn depth_is_the_longest_chain_below_a_command_with_a_cycle_counted_once() {
        let schema = schema(&request(&["fixture/adopter.proto"], None)).unwrap();
        let named = |name: &str| {
            schema
                .records
                .iter()
                .find(|r| r.name == name)
                .expect("the fixture names it")
        };
        // SetPath reaches Point and a map entry one level down, and Node,
        // which holds Node: two levels, and the walk ends.
        assert_eq!(depth(&[named("SetPath")], &schema.types), 2);
        assert_eq!(depth(&[named("GetPosition")], &schema.types), 0);
        assert_eq!(
            depth(&[named("GetPosition"), named("SetPath")], &schema.types),
            2
        );
        assert_eq!(depth(&[], &schema.types), 0);
        // The map's entry is read as a field type.
        assert!(
            schema
                .types
                .iter()
                .any(|t| t.name == "fixture.adopter.SetPath.TagsEntry")
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
    fn the_acknowledgement_is_read_apart_from_the_records() {
        // records.proto only imports broker.proto, and the acknowledgement
        // is read from the import all the same.
        let ack = schema(&request(&["fixture/records.proto"], None))
            .unwrap()
            .ack
            .expect("no acknowledgement");
        assert_eq!(ack.topic, "dcsbridge.broker.CommandAck");
        assert_eq!(ack.name, "CommandAck");
        assert_eq!(
            ack.fields,
            [
                field("request_id", 1, Kind::Uint64, Label::Optional),
                field("idempotency_key", 2, Kind::String, Label::Optional),
                field("outcome", 3, Kind::Enum, Label::Single),
                field("detail", 4, Kind::String, Label::Single),
            ]
        );
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
