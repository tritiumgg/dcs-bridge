//! The generated Lua files, written from the record rows, two per set.
//!
//! Each file builds its tables and returns them, and makes no `shim` call;
//! the driver that loads it registers them. The sim driver's file writes into
//! `DCSBridge.code`, and the hook driver's touches no global, so it loads
//! when no sim driver does. ADR 0038.
//!
//! The bridge's own records write the shipped pair, and each adopter package
//! writes a pair of its own that adds to the shipped tables. ADR 0041.

use std::fmt::Write;

use crate::records::{self, Field, Kind, Label, Record, Type};

/// The file the sim driver loads.
pub const SIM_FILE: &str = "SimDriver.gen.lua";
/// The file the hook driver loads.
pub const HOOK_FILE: &str = "HookDriver.gen.lua";

/// The package the bridge's own records are declared under.
const SHIPPED_PACKAGE: &str = "dcsbridge";

/// Which pair of files a record's rows go into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Set<'a> {
    /// The bridge's own records, in `dcsbridge` or a package under it.
    Shipped,
    /// An adopter's records, all in the one package named.
    Adopter(&'a str),
}

impl<'a> Set<'a> {
    /// The set a package's records belong to.
    pub fn of(package: &'a str) -> Self {
        let shipped = package
            .strip_prefix(SHIPPED_PACKAGE)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'));
        if shipped {
            Self::Shipped
        } else {
            Self::Adopter(package)
        }
    }

    /// The name of the set's file for one side.
    pub fn file_name(self, side: Side) -> String {
        match (self, side) {
            (Self::Shipped, Side::Sim) => SIM_FILE.to_owned(),
            (Self::Shipped, Side::Hook) => HOOK_FILE.to_owned(),
            (Self::Adopter(package), Side::Sim) => format!("{package}.sim.gen.lua"),
            (Self::Adopter(package), Side::Hook) => format!("{package}.hook.gen.lua"),
        }
    }
}

/// `Target`'s hook driver member. Unspecified reads as the sim driver.
const TARGET_HOOK_DRIVER: i32 = 2;
/// `RecordClass`'s command member, the class an inbound record carries.
const RECORD_CLASS_COMMAND: i32 = 3;

/// The two Lua states a file is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The mission scripting state, which reloads with each mission.
    Sim,
    /// The GUI state, which lives from DCS start to exit.
    Hook,
}

impl Side {
    fn of(record: &Record) -> Self {
        if record.target == TARGET_HOOK_DRIVER {
            Self::Hook
        } else {
            Self::Sim
        }
    }
}

/// What every file of one run shares.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// Every field type the run's records reach.
    pub types: &'a [Type],
    /// Every topic a record's `reply_to` names, across the whole run.
    pub replies: &'a [&'a str],
    /// The acknowledgement, whose emitter and `send_to` wrapper both shipped
    /// files carry, so a command handler on either side can answer with it.
    /// It is among `replies`. ADR 0040.
    pub ack: Option<&'a Record>,
    /// The hash each file carries.
    pub schema_sha256: &'a str,
}

/// Writes one set's file for one side over the set's records, keeping that
/// side's rows.
pub fn file(set: Set<'_>, side: Side, records: &[&Record], run: &Run<'_>) -> String {
    let schema_sha256 = run.schema_sha256;
    let mut rows: Vec<&Record> = records
        .iter()
        .copied()
        .filter(|r| Side::of(r) == side)
        .collect();
    rows.sort_by(|a, b| a.topic.cmp(&b.topic));

    let (table, mut out) = match (set, side) {
        (Set::Shipped, Side::Sim) => ("code", String::from(SIM_HEADER)),
        (Set::Shipped, Side::Hook) => ("gen", String::from(HOOK_HEADER)),
        (Set::Adopter(package), Side::Sim) => ("slice", adopter_sim_header(package)),
        (Set::Adopter(package), Side::Hook) => ("gen", adopter_hook_header(package)),
    };

    let _ = writeln!(out, "\n{table}.schema_sha256 = '{schema_sha256}'");

    map(
        &mut out,
        table,
        "topics",
        rows.iter().map(|r| (key(&r.name), quote(&r.topic))),
    );
    map(
        &mut out,
        table,
        "classes",
        rows.iter().map(|r| (topic_key(r), class(r.class))),
    );
    map(
        &mut out,
        table,
        "routes",
        rows.iter()
            .filter(|r| r.class == RECORD_CLASS_COMMAND)
            .map(|r| (topic_key(r), route(side))),
    );
    map(
        &mut out,
        table,
        "caps",
        rows.iter()
            .map(|r| (topic_key(r), capability(r.capability))),
    );

    let mut replies: Vec<&str> = rows.iter().filter_map(|r| r.reply_to.as_deref()).collect();
    replies.sort_unstable();
    replies.dedup();
    if replies.is_empty() {
        let _ = writeln!(out, "\n{table}.replies = {{}}");
    } else {
        let _ = writeln!(out, "\n{table}.replies = {{");
        for reply in replies {
            let _ = writeln!(out, "  {},", quote(reply));
        }
        out.push_str("}\n");
    }

    if table == "slice" {
        out.push_str(
            "\nfor name, topic in pairs(slice.topics) do\n  code.topics[name] = topic\nend\n",
        );
    }

    // An adopter's sim file adds to the emitters the shipped file made; every
    // other file makes its own.
    let owner = match side {
        Side::Sim => "code",
        Side::Hook => "gen",
    };
    let mut outbound: Vec<&Record> = rows
        .iter()
        .copied()
        .filter(|r| r.class != RECORD_CLASS_COMMAND)
        .collect();
    if let (Set::Shipped, Some(ack)) = (set, run.ack) {
        outbound.push(ack);
        outbound.sort_by(|a, b| a.topic.cmp(&b.topic));
    }
    let classed = writers(&mut out, &outbound, run.types);
    if table != "slice" {
        let _ = writeln!(out, "\n{owner}.emit = {{}}");
    }
    for record in &outbound {
        emitter(&mut out, owner, side, record, run);
    }
    let is_reply = |r: &&Record| run.replies.contains(&r.topic.as_str());
    for (sub, replies) in [("send", false), ("send_to", true)] {
        if table != "slice" {
            let _ = writeln!(out, "\n{owner}.{sub} = {{}}");
        }
        for record in outbound.iter().filter(|r| is_reply(r) == replies) {
            wrapper(&mut out, owner, sub, record, replies);
        }
    }

    if table != "slice" {
        let _ = writeln!(out, "\n{owner}.decode = {{}}");
    }
    let commands: Vec<&Record> = rows
        .iter()
        .copied()
        .filter(|r| r.class == RECORD_CLASS_COMMAND)
        .collect();
    decoders(&mut out, owner, &commands, run.types, &classed);

    let _ = writeln!(out, "\nreturn {table}");
    out
}

/// The names a parameter may not take, because the emitter reads them
/// itself: Lua's keywords, and the globals, locals and tables a generated
/// file uses. ADR 0040.
const RESERVED: [&str; 27] = [
    "and",
    "break",
    "do",
    "else",
    "elseif",
    "end",
    "false",
    "for",
    "function",
    "if",
    "in",
    "local",
    "nil",
    "not",
    "or",
    "repeat",
    "return",
    "then",
    "true",
    "until",
    "while",
    "DCSBridge",
    "ipairs",
    "shim",
    "put",
    "code",
    "gen",
];

/// The connection id a reply's emitter takes first.
const CONN_ID: &str = "conn_id";

/// One emitter: open the record, one put per field in declaration order, and
/// return what `commit` returns. A reply's opens with `begin_to` and takes a
/// connection id first. ADR 0040.
fn emitter(out: &mut String, owner: &str, side: Side, record: &Record, run: &Run<'_>) {
    let reply = run.replies.contains(&record.topic.as_str());
    let names = params(record, reply);
    out.push('\n');
    param_docs(out, record, &names, reply);
    out.push_str("---@return boolean queued what commit returns\n");
    let _ = writeln!(
        out,
        "{} = function({})",
        member(owner, "emit", &record.name),
        names.join(", ")
    );
    if side == Side::Sim {
        out.push_str("  local shim = DCSBridge.shim\n");
    }
    if reply {
        let _ = writeln!(out, "  shim.begin_to({CONN_ID}, {})", quote(&record.topic));
    } else {
        let _ = writeln!(out, "  shim.begin({})", quote(&record.topic));
    }
    let fields = &names[usize::from(reply)..];
    for (field, name) in record.fields.iter().zip(fields) {
        field_puts(out, field, name, 1);
    }
    out.push_str("  return shim.commit()\nend\n");
}

/// One send wrapper: the emitter's parameters, handed with the emitter to
/// the `buffer` the runtime puts on the table, which the drain later calls.
/// The wrapper reads `buffer` when called, so the runtime may set it after
/// the file loads. ADR 0040.
fn wrapper(out: &mut String, owner: &str, sub: &str, record: &Record, reply: bool) {
    let names = params(record, reply);
    let args = names.join(", ");
    out.push('\n');
    param_docs(out, record, &names, reply);
    let _ = writeln!(
        out,
        "{} = function({args})",
        member(owner, sub, &record.name)
    );
    let emit = member(owner, "emit", &record.name);
    let separator = if args.is_empty() { "" } else { ", " };
    let _ = writeln!(out, "  return {owner}.buffer({emit}{separator}{args})\nend");
}

/// The emitter's parameters: the connection id for a reply, then one per
/// field. A name that is reserved or already taken gains a `_` until it is
/// neither.
fn params(record: &Record, reply: bool) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if reply {
        names.push(CONN_ID.to_owned());
    }
    for field in &record.fields {
        let mut name = field.name.clone();
        while RESERVED.contains(&name.as_str()) || names.contains(&name) {
            name.push('_');
        }
        names.push(name);
    }
    names
}

/// A `---@param` line per parameter, typed from its field.
fn param_docs(out: &mut String, record: &Record, names: &[String], reply: bool) {
    if reply {
        let _ = writeln!(out, "---@param {CONN_ID} integer");
    }
    for (field, name) in record.fields.iter().zip(&names[usize::from(reply)..]) {
        let _ = writeln!(out, "---@param {name} {}", lua_type(field));
    }
}

/// The LuaLS type of a field's argument. A field whose absence differs from
/// zero, an `optional` scalar or a message, may be nil; a scalar that reads
/// as zero when absent is marked required, though nil writes no put.
fn lua_type(field: &Field) -> String {
    let (base, nullable) = match &field.kind {
        Kind::Int32
        | Kind::Int64
        | Kind::Uint32
        | Kind::Uint64
        | Kind::Enum
        | Kind::Sint32
        | Kind::Sint64
        | Kind::Fixed32
        | Kind::Fixed64
        | Kind::Sfixed32
        | Kind::Sfixed64 => ("integer", false),
        Kind::Float | Kind::Double => ("number", false),
        Kind::String => ("string", false),
        Kind::Boolean => ("boolean", false),
        Kind::Message(name) => (name.as_str(), true),
        Kind::Map(_) | Kind::Group => ("table", false),
    };
    match field.label {
        Label::Repeated => format!("{base}[]"),
        Label::Optional => format!("{base}?"),
        Label::Single if nullable => format!("{base}?"),
        Label::Single => base.to_owned(),
    }
}

/// The puts for one field holding `value`, skipped when it is nil. A
/// repeated field puts each element in array order.
fn field_puts(out: &mut String, field: &Field, value: &str, depth: usize) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(out, "{pad}if {value} ~= nil then");
    if field.label == Label::Repeated {
        let _ = writeln!(out, "{pad}  for _, v in ipairs({value}) do");
        put(out, field, "v", depth + 2);
        let _ = writeln!(out, "{pad}  end");
    } else {
        put(out, field, value, depth + 1);
    }
    let _ = writeln!(out, "{pad}end");
}

/// One value's put: a scalar's call, or a message's fields between
/// `message` and `end_message`, written by the field type's writer.
fn put(out: &mut String, field: &Field, value: &str, depth: usize) {
    let pad = "  ".repeat(depth);
    let number = field.number;
    if let Kind::Message(name) = &field.kind {
        let _ = writeln!(out, "{pad}shim.message({number})");
        let _ = writeln!(out, "{pad}put[{}](shim, {value})", quote(name));
        let _ = writeln!(out, "{pad}shim.end_message()");
    } else if let Some(call) = field.kind.put() {
        let _ = writeln!(out, "{pad}shim.{call}({number}, {value})");
    }
    // A kind with no put is refused before any file is written, so an
    // emitter never reaches one.
}

/// Every field type the records' fields reach, directly or through another
/// field type, sorted by full name.
fn reached<'a>(records: &[&Record], types: &'a [Type]) -> Vec<&'a Type> {
    let mut reached: Vec<&Type> = Vec::new();
    let mut pending: Vec<&Field> = records.iter().flat_map(|r| &r.fields).collect();
    while let Some(field) = pending.pop() {
        let Some(name) = field.kind.message() else {
            continue;
        };
        if reached.iter().any(|t| t.name == name) {
            continue;
        }
        if let Some(found) = types.iter().find(|t| t.name == name) {
            reached.push(found);
            pending.extend(&found.fields);
        }
    }
    reached.sort_by(|a, b| a.name.cmp(&b.name));
    reached
}

/// A class per field type, by full name: two packages may each declare a
/// Point, and LuaLS merges classes of one name. ADR 0040.
fn classes(out: &mut String, types: &[&Type]) {
    for found in types {
        let _ = writeln!(out, "\n---@class {}", found.name);
        for field in &found.fields {
            let _ = writeln!(out, "---@field {} {}", field.name, lua_type(field));
        }
    }
}

/// A writer per field type the emitters reach, keyed by full name, after a
/// class for each. A writer reads its fields from a table keyed by field
/// name. Returns the types it wrote a class for.
fn writers<'a>(out: &mut String, records: &[&Record], types: &'a [Type]) -> Vec<&'a Type> {
    let reached = reached(records, types);
    if reached.is_empty() {
        return reached;
    }
    classes(out, &reached);
    out.push_str("\nlocal put = {}\n");
    for found in &reached {
        let _ = writeln!(out, "\n---@param value {}", found.name);
        let _ = writeln!(out, "put[{}] = function(shim, value)", quote(&found.name));
        for field in &found.fields {
            field_puts(out, field, &index("value", &field.name), 1);
        }
        out.push_str("end\n");
    }
    reached
}

/// The wire reader the decoders share, written into each file that holds a
/// command. ADR 0044.
const WIRE: &str = include_str!("wire.lua");

/// How many messages deep a recursive field type may nest past its first
/// level, over the depth the schema needs. ADR 0045.
pub const DEPTH_MARGIN: usize = 8;

/// The most elements one repeated field or map holds in one message.
/// ADR 0045.
pub const MAX_ELEMENTS: usize = 4096;

/// A decoder per command, keyed by topic: the command's shape, a table from
/// field number to field, handed to the reader with the body, and a shape
/// per field type the commands reach. `classed` names the field types a
/// class was already written for. ADR 0043.
fn decoders(
    out: &mut String,
    owner: &str,
    commands: &[&Record],
    types: &[Type],
    classed: &[&Type],
) {
    if commands.is_empty() {
        return;
    }
    let reached = reached(commands, types);
    let depth = records::depth(commands, types) + DEPTH_MARGIN;
    let _ = writeln!(
        out,
        "\n-- The bounds every decode in this file holds to: {DEPTH_MARGIN} levels past \
         the deepest\n-- the schema nests, and {MAX_ELEMENTS} elements in one repeated \
         field or map.\n-- ADR 0045.\nlocal MAX_DEPTH = {depth}\nlocal MAX_ELEMENTS = \
         {MAX_ELEMENTS}\n"
    );
    out.push_str(WIRE);
    for record in commands {
        shape(out, &record.topic, &record.fields);
    }
    for found in &reached {
        shape(out, &found.name, &found.fields);
    }
    let unclassed: Vec<&Type> = reached
        .into_iter()
        .filter(|t| !classed.iter().any(|c| c.name == t.name))
        .collect();
    classes(out, &unclassed);
    for record in commands {
        let topic = quote(&record.topic);
        let _ = writeln!(out, "\n---@class {}", record.topic);
        for field in &record.fields {
            let _ = writeln!(out, "---@field {} {}", field.name, lua_type(field));
        }
        let _ = writeln!(
            out,
            "\n---@param bytes string\n---@return {}\n{owner}.decode[{topic}] = function(bytes)\n  \
             return message(shape[{topic}], bytes, 1, #bytes, 0)\nend",
            record.topic
        );
    }
}

/// One message's shape: a row per field the reader decodes, keyed by field
/// number. A field with no row is skipped like an unknown one.
fn shape(out: &mut String, name: &str, fields: &[Field]) {
    let rows: Vec<String> = fields
        .iter()
        .filter_map(|field| {
            let kind = decoded(&field.kind)?;
            let mut row = format!("name = {}, kind = '{kind}'", quote(&field.name));
            match field.label {
                Label::Optional => row.push_str(", optional = true"),
                Label::Repeated => row.push_str(", repeated = true"),
                Label::Single => {}
            }
            if let Kind::Message(name) = &field.kind {
                let _ = write!(row, ", type = {}", quote(name));
            }
            Some(format!("  [{}] = {{ {row} }},", field.number))
        })
        .collect();
    if rows.is_empty() {
        let _ = writeln!(out, "\nshape[{}] = {{}}", quote(name));
    } else {
        let _ = writeln!(
            out,
            "\nshape[{}] = {{\n{}\n}}",
            quote(name),
            rows.join("\n")
        );
    }
}

/// The reader's name for a kind it decodes.
fn decoded(kind: &Kind) -> Option<&'static str> {
    match kind {
        Kind::Int32 => Some("int32"),
        Kind::Int64 => Some("int64"),
        Kind::Uint32 => Some("uint32"),
        Kind::Uint64 => Some("uint64"),
        Kind::Enum => Some("enum"),
        Kind::Boolean => Some("bool"),
        Kind::Double => Some("double"),
        Kind::Message(_) => Some("message"),
        Kind::String => Some("string"),
        _ => None,
    }
}

/// `table.name`, or `table['name']` for a name Lua reserves.
fn index(table: &str, name: &str) -> String {
    let key = key(name);
    if key.starts_with('[') {
        format!("{table}{key}")
    } else {
        format!("{table}.{key}")
    }
}

/// `owner.sub.name`, bracketed the same way.
fn member(owner: &str, sub: &str, name: &str) -> String {
    index(&format!("{owner}.{sub}"), name)
}

fn adopter_sim_header(package: &str) -> String {
    format!(
        "\
-- Generated by protoc-gen-dcsbridge-lua from the schema of package
-- {package}. Do not edit it: change the schema and generate it again.
--
-- The sim driver's slice for this package. It loads after the shipped
-- SimDriver.gen.lua and adds its rows to DCSBridge.code by key, and the sim
-- driver registers the tables it returns. ADR 0041.

local code = DCSBridge.code
local slice = {{}}
"
    )
}

fn adopter_hook_header(package: &str) -> String {
    format!(
        "\
-- Generated by protoc-gen-dcsbridge-lua from the schema of package
-- {package}. Do not edit it: change the schema and generate it again.
--
-- The hook driver's tables for this package. Like HookDriver.gen.lua, they
-- stay out of DCSBridge.code, and the hook driver registers them at DCS
-- start. ADR 0041.
--
-- The hook driver loads it with the shim as its argument, which its
-- emitters write through. ADR 0040.

local gen = {{}}
local shim = ...
"
    )
}

const SIM_HEADER: &str = "\
-- Generated by protoc-gen-dcsbridge-lua from the bridge schema. Do not edit
-- it: change the schema and generate it again.
--
-- The sim driver's tables. Each load assigns them into DCSBridge.code fresh,
-- so a reload replaces them, and the sim driver registers them through the
-- shim. ADR 0038.

local code = DCSBridge.code
";

const HOOK_HEADER: &str = "\
-- Generated by protoc-gen-dcsbridge-lua from the bridge schema. Do not edit
-- it: change the schema and generate it again.
--
-- The hook driver's tables. They stay out of DCSBridge.code, so a
-- hook-driver topic works when no sim driver is loaded, and the hook driver
-- registers them through the shim at DCS start. ADR 0038.
--
-- The hook driver loads it with the shim as its argument, which its
-- emitters write through. ADR 0040.

local gen = {}
local shim = ...
";

fn map(out: &mut String, table: &str, name: &str, rows: impl Iterator<Item = (String, String)>) {
    let rows: Vec<_> = rows.collect();
    if rows.is_empty() {
        let _ = writeln!(out, "\n{table}.{name} = {{}}");
        return;
    }
    let _ = writeln!(out, "\n{table}.{name} = {{");
    for (key, value) in rows {
        let _ = writeln!(out, "  {key} = {value},");
    }
    out.push_str("}\n");
}

/// Names and topics are protobuf identifiers joined by dots, so a single
/// quote needs no escape inside them.
fn quote(text: &str) -> String {
    format!("'{text}'")
}

fn topic_key(record: &Record) -> String {
    format!("[{}]", quote(&record.topic))
}

/// A message name is a Lua name unless it is a Lua keyword.
fn key(name: &str) -> String {
    const KEYWORDS: [&str; 21] = [
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
        "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ];
    if KEYWORDS.contains(&name) {
        format!("[{}]", quote(name))
    } else {
        name.to_owned()
    }
}

fn class(number: i32) -> String {
    match number {
        1 => quote("durable"),
        2 => quote("lossy"),
        3 => quote("command"),
        4 => quote("lifecycle"),
        other => other.to_string(),
    }
}

/// The bridge's own capabilities by name; one numbered past them, a
/// built-in set's or an adopter's, by its number.
fn capability(number: i32) -> String {
    match number {
        1 => quote("read"),
        2 => quote("command"),
        3 => quote("reload"),
        other => other.to_string(),
    }
}

fn route(side: Side) -> String {
    match side {
        Side::Sim => quote("sim_driver"),
        Side::Hook => quote("hook_driver"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(topic: &str, class: i32, target: i32, capability: i32) -> Record {
        Record {
            topic: topic.into(),
            name: topic.rsplit('.').next().unwrap().into(),
            class,
            target,
            capability,
            reply_to: None,
            fields: Vec::new(),
        }
    }

    fn run(schema_sha256: &str) -> Run<'_> {
        Run {
            types: &[],
            replies: &[],
            ack: None,
            schema_sha256,
        }
    }

    fn shipped(side: Side, records: &[Record], schema_sha256: &str) -> String {
        let records: Vec<_> = records.iter().collect();
        file(Set::Shipped, side, &records, &run(schema_sha256))
    }

    fn field(name: &str, number: u32, kind: Kind, label: Label) -> Field {
        Field {
            name: name.into(),
            number,
            kind,
            label,
        }
    }

    fn with(mut record: Record, fields: Vec<Field>) -> Record {
        record.fields = fields;
        record
    }

    #[test]
    fn an_emitter_puts_each_field_in_declaration_order_and_each_element_once() {
        let point = Kind::Message("a.Point".into());
        let impact = with(
            record("a.RocketImpact", 2, 0, 1),
            vec![
                field("weapon", 3, Kind::String, Label::Single),
                field("hit_ids", 1, Kind::Int32, Label::Repeated),
                field("craters", 2, point.clone(), Label::Repeated),
                field("yield", 4, Kind::Double, Label::Optional),
                field("at", 5, point, Label::Single),
            ],
        );
        let types = [Type {
            name: "a.Point".into(),
            fields: vec![field("x", 1, Kind::Double, Label::Single)],
        }];
        let run = Run {
            types: &types,
            ..run("h")
        };
        let text = file(Set::Shipped, Side::Sim, &[&impact], &run);
        let emitter = "
code.emit.RocketImpact = function(weapon, hit_ids, craters, yield, at)
  local shim = DCSBridge.shim
  shim.begin('a.RocketImpact')
  if weapon ~= nil then
    shim.string(3, weapon)
  end
  if hit_ids ~= nil then
    for _, v in ipairs(hit_ids) do
      shim.integer(1, v)
    end
  end
  if craters ~= nil then
    for _, v in ipairs(craters) do
      shim.message(2)
      put['a.Point'](shim, v)
      shim.end_message()
    end
  end
  if yield ~= nil then
    shim.double(4, yield)
  end
  if at ~= nil then
    shim.message(5)
    put['a.Point'](shim, at)
    shim.end_message()
  end
  return shim.commit()
end
";
        assert!(text.contains(emitter), "{text}");
        let writer = "
put['a.Point'] = function(shim, value)
  if value.x ~= nil then
    shim.double(1, value.x)
  end
end
";
        assert!(text.contains(writer), "{text}");
        assert!(
            text.find("local put = {}").unwrap() < text.find(emitter).unwrap(),
            "{text}"
        );
    }

    #[test]
    fn a_reply_takes_a_connection_id_first_and_opens_with_begin_to() {
        let position = with(
            record("a.Position", 1, 0, 1),
            vec![field("conn_id", 1, Kind::Int32, Label::Single)],
        );
        let replies = ["a.Position"];
        let run = Run {
            replies: &replies,
            ..run("h")
        };
        let text = file(Set::Shipped, Side::Sim, &[&position], &run);
        assert!(
            text.contains("code.emit.Position = function(conn_id, conn_id_)\n"),
            "{text}"
        );
        assert!(
            text.contains("  shim.begin_to(conn_id, 'a.Position')\n"),
            "{text}"
        );
        assert!(text.contains("shim.integer(1, conn_id_)"), "{text}");
    }

    #[test]
    fn a_command_has_no_emitter_and_a_hook_emitter_uses_the_chunk_argument() {
        let records = [
            record("a.SetMark", 3, 2, 2),
            with(
                record("a.Chat", 1, 2, 1),
                vec![field("end", 1, Kind::String, Label::Single)],
            ),
        ];
        let hook = shipped(Side::Hook, &records, "h");
        assert!(hook.contains("\nlocal shim = ...\n"), "{hook}");
        assert!(!hook.contains("gen.emit.SetMark"), "{hook}");
        assert!(
            hook.contains("\ngen.emit.Chat = function(end_)\n  shim.begin('a.Chat')\n"),
            "{hook}"
        );
    }

    #[test]
    fn an_emitter_and_each_field_type_carry_luals_annotations() {
        let point = Kind::Message("a.Point".into());
        let position = with(
            record("a.Position", 1, 0, 1),
            vec![
                field("request_id", 1, Kind::Int32, Label::Single),
                field("note", 2, Kind::String, Label::Optional),
                field("path", 3, point.clone(), Label::Repeated),
                field("at", 4, point, Label::Single),
            ],
        );
        let types = [Type {
            name: "a.Point".into(),
            fields: vec![
                field("x", 1, Kind::Double, Label::Single),
                field("hit", 2, Kind::Boolean, Label::Single),
            ],
        }];
        let replies = ["a.Position"];
        let run = Run {
            types: &types,
            replies: &replies,
            ..run("h")
        };
        let text = file(Set::Shipped, Side::Sim, &[&position], &run);
        let docs = "
---@param conn_id integer
---@param request_id integer
---@param note string?
---@param path a.Point[]
---@param at a.Point?
---@return boolean queued what commit returns
code.emit.Position = function(conn_id, request_id, note, path, at)
";
        assert!(text.contains(docs), "{text}");
        assert!(
            text.contains("\n---@class a.Point\n---@field x number\n---@field hit boolean\n"),
            "{text}"
        );
        assert!(
            text.contains("\n---@param value a.Point\nput['a.Point'] = function(shim, value)\n"),
            "{text}"
        );
    }

    #[test]
    fn a_fan_out_record_has_a_send_wrapper_and_a_reply_a_send_to() {
        let records = [
            with(
                record("a.Impact", 2, 0, 1),
                vec![field("weapon", 1, Kind::String, Label::Single)],
            ),
            with(
                record("a.Position", 1, 0, 1),
                vec![field("request_id", 1, Kind::Int32, Label::Single)],
            ),
            record("a.Empty", 2, 0, 1),
        ];
        let records: Vec<_> = records.iter().collect();
        let replies = ["a.Position"];
        let run = Run {
            replies: &replies,
            ..run("h")
        };
        let text = file(Set::Shipped, Side::Sim, &records, &run);
        let send = "
---@param weapon string
code.send.Impact = function(weapon)
  return code.buffer(code.emit.Impact, weapon)
end
";
        assert!(text.contains(send), "{text}");
        let send_to = "
---@param conn_id integer
---@param request_id integer
code.send_to.Position = function(conn_id, request_id)
  return code.buffer(code.emit.Position, conn_id, request_id)
end
";
        assert!(text.contains(send_to), "{text}");
        assert!(
            text.contains(
                "code.send.Empty = function()\n  return code.buffer(code.emit.Empty)\nend"
            ),
            "{text}"
        );
        assert!(!text.contains("code.send.Position"), "{text}");
        assert!(!text.contains("code.send_to.Impact"), "{text}");
        assert!(text.contains("\ncode.send = {}\n"), "{text}");
        assert!(text.contains("\ncode.send_to = {}\n"), "{text}");

        let adopter = file(Set::Adopter("a"), Side::Sim, &records, &run);
        assert!(!adopter.contains("code.send = {}"), "{adopter}");
        assert!(adopter.contains("code.send.Impact = function"), "{adopter}");
    }

    #[test]
    fn both_shipped_files_carry_the_acknowledgement_and_no_row_for_it() {
        let ack = with(
            record("dcsbridge.broker.CommandAck", 1, 0, 0),
            vec![field("outcome", 3, Kind::Int32, Label::Single)],
        );
        let replies = ["dcsbridge.broker.CommandAck"];
        let run = Run {
            replies: &replies,
            ack: Some(&ack),
            ..run("h")
        };
        for (side, owner) in [(Side::Sim, "code"), (Side::Hook, "gen")] {
            let text = file(Set::Shipped, side, &[], &run);
            assert!(
                text.contains(&format!(
                    "{owner}.emit.CommandAck = function(conn_id, outcome)\n"
                )),
                "{text}"
            );
            assert!(
                text.contains("shim.begin_to(conn_id, 'dcsbridge.broker.CommandAck')"),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "{owner}.send_to.CommandAck = function(conn_id, outcome)\n"
                )),
                "{text}"
            );
            assert!(text.contains(&format!("{owner}.topics = {{}}")), "{text}");
            assert!(text.contains(&format!("{owner}.classes = {{}}")), "{text}");
        }
        let adopter = file(Set::Adopter("a"), Side::Sim, &[], &run);
        assert!(!adopter.contains("CommandAck"), "{adopter}");
    }

    #[test]
    fn a_command_has_a_decoder_and_only_a_file_with_one_carries_the_reader() {
        let records = [record("a.SetMark", 3, 2, 2), record("a.Spotted", 2, 0, 1)];
        let hook = shipped(Side::Hook, &records, "h");
        assert!(hook.contains("\ngen.decode = {}\n"), "{hook}");
        assert!(hook.contains("\nlocal function varint("), "{hook}");
        assert!(hook.contains("\nshape['a.SetMark'] = {}\n"), "{hook}");
        let decoder = "
---@param bytes string
---@return a.SetMark
gen.decode['a.SetMark'] = function(bytes)
  return message(shape['a.SetMark'], bytes, 1, #bytes, 0)
end
";
        assert!(hook.contains(decoder), "{hook}");
        assert!(
            hook.find("\nlocal function message(").unwrap() < hook.find(decoder).unwrap(),
            "{hook}"
        );

        let sim = shipped(Side::Sim, &records, "h");
        assert!(sim.contains("\ncode.decode = {}\n"), "{sim}");
        assert!(
            !sim.contains("varint"),
            "a file with no command has a reader: {sim}"
        );

        let refs: Vec<_> = records.iter().collect();
        let adopter = file(Set::Adopter("a"), Side::Sim, &refs, &run("h"));
        assert!(!adopter.contains("decode = {}"), "{adopter}");
    }

    #[test]
    fn a_decoder_shape_has_a_row_per_field_it_reads_and_a_class_names_every_field() {
        let command = with(
            record("a.SetMark", 3, 0, 2),
            vec![
                field("label", 2, Kind::String, Label::Single),
                field("serial", 1, Kind::Uint64, Label::Single),
                field("note", 4, Kind::String, Label::Optional),
                field("group", 3, Kind::Group, Label::Single),
                field("side", 5, Kind::Enum, Label::Single),
                field("on", 6, Kind::Boolean, Label::Single),
                field("at", 7, Kind::Double, Label::Single),
            ],
        );
        let text = shipped(Side::Sim, &[command], "h");
        let shape = "
shape['a.SetMark'] = {
  [2] = { name = 'label', kind = 'string' },
  [1] = { name = 'serial', kind = 'uint64' },
  [4] = { name = 'note', kind = 'string', optional = true },
  [5] = { name = 'side', kind = 'enum' },
  [6] = { name = 'on', kind = 'bool' },
  [7] = { name = 'at', kind = 'double' },
}
";
        assert!(text.contains(shape), "{text}");
        let class = "
---@class a.SetMark
---@field label string
---@field serial integer
---@field note string?
---@field group table
---@field side integer
---@field on boolean
---@field at number
";
        assert!(text.contains(class), "{text}");
    }

    #[test]
    fn a_decoder_reaches_its_field_types_under_bounds_from_the_schema() {
        let point = Kind::Message("a.Point".into());
        let node = Kind::Message("a.Node".into());
        let records = [
            with(
                record("a.SetPath", 3, 0, 2),
                vec![
                    field("waypoints", 1, point.clone(), Label::Repeated),
                    field("tree", 2, node.clone(), Label::Single),
                    field("ids", 3, Kind::Int32, Label::Repeated),
                ],
            ),
            with(
                record("a.Spotted", 2, 0, 1),
                vec![field("at", 1, point, Label::Single)],
            ),
        ];
        let types = [
            Type {
                name: "a.Node".into(),
                fields: vec![field("children", 1, node, Label::Repeated)],
            },
            Type {
                name: "a.Point".into(),
                fields: vec![field("x", 1, Kind::Double, Label::Single)],
            },
        ];
        let run = Run {
            types: &types,
            ..run("h")
        };
        let refs: Vec<_> = records.iter().collect();
        let text = file(Set::Shipped, Side::Sim, &refs, &run);
        // Node holds Node: two levels, then the margin.
        assert!(
            text.contains(&format!(
                "\nlocal MAX_DEPTH = {}\nlocal MAX_ELEMENTS = 4096\n",
                2 + DEPTH_MARGIN
            )),
            "{text}"
        );
        let command = "
shape['a.SetPath'] = {
  [1] = { name = 'waypoints', kind = 'message', repeated = true, type = 'a.Point' },
  [2] = { name = 'tree', kind = 'message', type = 'a.Node' },
  [3] = { name = 'ids', kind = 'int32', repeated = true },
}
";
        assert!(text.contains(command), "{text}");
        assert!(
            text.contains(
                "\nshape['a.Node'] = {\n  [1] = { name = 'children', kind = 'message', \
                 repeated = true, type = 'a.Node' },\n}\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("\nshape['a.Point'] = {\n  [1] = { name = 'x', kind = 'double' },\n}\n"),
            "{text}"
        );
        // Point has a class from the writers, and Node one from the decoders.
        assert_eq!(text.matches("---@class a.Point\n").count(), 1, "{text}");
        assert_eq!(text.matches("---@class a.Node\n").count(), 1, "{text}");
        assert!(
            text.find("local MAX_DEPTH").unwrap() < text.find("local function message(").unwrap(),
            "{text}"
        );
    }

    #[test]
    fn a_reserved_name_is_renamed_and_a_keyword_key_bracketed() {
        let record = with(
            record("a.end", 1, 0, 1),
            vec![
                field("shim", 1, Kind::Int32, Label::Single),
                field("shim_", 2, Kind::Int32, Label::Single),
            ],
        );
        let text = shipped(Side::Sim, &[record], "h");
        assert!(
            text.contains("code.emit['end'] = function(shim_, shim__)\n"),
            "{text}"
        );
    }

    fn requesting(mut record: Record, reply: &str) -> Record {
        record.reply_to = Some(reply.into());
        record
    }

    #[test]
    fn an_outbound_record_has_a_class_and_a_capability_and_no_route() {
        let text = shipped(Side::Sim, &[record("a.Spotted", 2, 0, 1)], "h");
        assert!(
            text.contains("code.topics = {\n  Spotted = 'a.Spotted',\n}"),
            "{text}"
        );
        assert!(
            text.contains("code.classes = {\n  ['a.Spotted'] = 'lossy',\n}"),
            "{text}"
        );
        assert!(
            text.contains("code.caps = {\n  ['a.Spotted'] = 'read',\n}"),
            "{text}"
        );
        assert!(text.contains("code.routes = {}\n"), "{text}");
    }

    #[test]
    fn a_command_routes_to_its_own_side() {
        let records = [record("a.SetFlag", 3, 1, 2), record("a.SetMark", 3, 2, 2)];
        let sim = shipped(Side::Sim, &records, "h");
        let hook = shipped(Side::Hook, &records, "h");
        assert!(
            sim.contains("code.routes = {\n  ['a.SetFlag'] = 'sim_driver',\n}"),
            "{sim}"
        );
        assert!(
            hook.contains("gen.routes = {\n  ['a.SetMark'] = 'hook_driver',\n}"),
            "{hook}"
        );
    }

    #[test]
    fn a_hook_record_lands_only_in_the_hook_file() {
        let records = [record("a.Chat", 1, 2, 1)];
        let sim = shipped(Side::Sim, &records, "h");
        assert!(!sim.contains("a.Chat"), "{sim}");
        let hook = shipped(Side::Hook, &records, "h");
        assert!(hook.contains("['a.Chat'] = 'durable'"), "{hook}");
        let mut code = hook.lines().filter(|line| !line.starts_with("--"));
        assert!(
            code.all(|line| !line.contains("DCSBridge")),
            "the hook file names a global: {hook}"
        );
        assert!(hook.ends_with("\nreturn gen\n"), "{hook}");
    }

    #[test]
    fn two_requests_naming_one_reply_list_it_once() {
        let records = [
            requesting(record("a.GetA", 3, 0, 2), "a.Reply"),
            requesting(record("a.GetB", 3, 0, 2), "a.Reply"),
            record("a.Reply", 1, 0, 1),
        ];
        let text = shipped(Side::Sim, &records, "h");
        assert!(text.contains("code.replies = {\n  'a.Reply',\n}"), "{text}");
    }

    #[test]
    fn rows_sort_by_topic_and_the_text_is_stable() {
        let records = [record("b.Zulu", 4, 0, 3), record("a.Alpha", 1, 0, 100)];
        let text = shipped(Side::Sim, &records, "h");
        let alpha = text.find("['a.Alpha'] = 'durable'").unwrap();
        let zulu = text.find("['b.Zulu'] = 'lifecycle'").unwrap();
        assert!(alpha < zulu, "{text}");
        assert!(
            text.contains("['a.Alpha'] = 100,"),
            "a capability past the enum: {text}"
        );
        assert_eq!(text, shipped(Side::Sim, &records, "h"));
    }

    #[test]
    fn the_sim_file_assigns_into_dcsbridge_code_and_carries_the_hash() {
        let text = shipped(Side::Sim, &[], "abc123");
        assert!(text.contains("\nlocal code = DCSBridge.code\n"), "{text}");
        assert!(text.contains("\ncode.schema_sha256 = 'abc123'\n"), "{text}");
        assert!(text.ends_with("\nreturn code\n"), "{text}");
    }

    #[test]
    fn a_keyword_name_is_bracketed() {
        let text = shipped(Side::Sim, &[record("a.end", 1, 0, 1)], "h");
        assert!(text.contains("  ['end'] = 'a.end',"), "{text}");
    }

    #[test]
    fn a_package_under_dcsbridge_is_shipped_and_any_other_is_an_adopter() {
        assert_eq!(Set::of("dcsbridge"), Set::Shipped);
        assert_eq!(Set::of("dcsbridge.builtin.sim"), Set::Shipped);
        assert_eq!(Set::of("dcsbridgex"), Set::Adopter("dcsbridgex"));
        assert_eq!(Set::of("acme.ops"), Set::Adopter("acme.ops"));
        assert_eq!(Set::Shipped.file_name(Side::Hook), HOOK_FILE);
        assert_eq!(
            Set::of("acme.ops").file_name(Side::Sim),
            "acme.ops.sim.gen.lua"
        );
    }

    #[test]
    fn an_adopter_sim_file_returns_its_slice_and_adds_its_topics_to_the_code() {
        let spotted = record("acme.Spotted", 2, 0, 1);
        let text = file(Set::Adopter("acme"), Side::Sim, &[&spotted], &run("h"));
        assert!(text.contains("\nlocal slice = {}\n"), "{text}");
        assert!(
            text.contains("slice.topics = {\n  Spotted = 'acme.Spotted',\n}"),
            "{text}"
        );
        assert!(
            text.contains(
                "for name, topic in pairs(slice.topics) do\n  code.topics[name] = topic\nend"
            ),
            "{text}"
        );
        assert!(text.ends_with("\nreturn slice\n"), "{text}");
        assert!(
            !text.contains("code.topics = "),
            "it replaces the shipped topics: {text}"
        );
    }
}
