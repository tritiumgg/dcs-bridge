//! The generated files over the shipped schema and over an adopter's,
//! compared byte for byte with the copies under `tests/golden`.
//!
//! The adopter's schema is `tests/proto/fixture/adopter.proto`. The shipped
//! one has no repeated field, reply or hook-side event, and the adopter's
//! has each, so its copies show what the shipped ones cannot.
//!
//! A change to the generator or to the schema shows up here as a diff a
//! reviewer reads. When the change is meant, rewrite the copies:
//!
//! ```sh
//! DCSBRIDGE_UPDATE_GOLDEN=1 cargo test -p protoc-gen-dcsbridge-lua --test golden
//! ```
//!
//! The hash the files carry is of `tests/abc.pb`, three fixed bytes, so the
//! copies move with the schema's records and options and not with its
//! comments. `tests/lua/generated.lua` loads the same copies.

use std::fs;
use std::path::Path;

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

/// Every file under `proto/`, as buf names them for generation.
const SCHEMA: [&str; 3] = [
    "dcsbridge/broker/broker.proto",
    "dcsbridge/builtin/sim/sim.proto",
    "dcsbridge/hook/hook.proto",
];

/// The adopter's schema, which imports the shipped tree.
const ADOPTER: [&str; 1] = ["fixture/adopter.proto"];

/// The request buf sends over `files`: those named for generation, and every
/// import in `proto_file` before them.
fn request(root: &Path, files: &[&str]) -> Vec<u8> {
    let mut compiler = protox::Compiler::new([root.join("tests/proto"), root.join("../../proto")])
        .expect("the include paths resolve");
    compiler.include_imports(true);
    compiler.open_files(files).expect("the schema compiles");
    let set = Set::decode(compiler.encode_file_descriptor_set().as_slice())
        .expect("protox wrote a descriptor set");
    Request {
        file_to_generate: files.iter().map(|f| (*f).to_owned()).collect(),
        parameter: Some(format!("schema={}", root.join("tests/abc.pb").display())),
        proto_file: set.file,
    }
    .encode_to_vec()
}

/// Names the first line that differs, so a failure reads as a diff rather
/// than as two whole files.
fn first_difference(expected: &str, actual: &str) -> Option<String> {
    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    for line in 1.. {
        match (expected_lines.next(), actual_lines.next()) {
            (None, None) => return None,
            (e, a) if e == a => {}
            (e, a) => {
                return Some(format!(
                    "line {line}\n  golden:    {}\n  generated: {}",
                    e.unwrap_or("<end of file>"),
                    a.unwrap_or("<end of file>")
                ));
            }
        }
    }
    unreachable!("the loop returns at the end of both files")
}

#[test]
fn the_shipped_and_adopter_schemas_generate_the_golden_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for (schema, names) in [
        (&SCHEMA[..], ["SimDriver.gen.lua", "HookDriver.gen.lua"]),
        (
            &ADOPTER[..],
            [
                "fixture.adopter.sim.gen.lua",
                "fixture.adopter.hook.gen.lua",
            ],
        ),
    ] {
        let response = protoc_gen_dcsbridge_lua::respond(&request(root, schema));
        assert_eq!(response.error, None);
        let written: Vec<_> = response.file.iter().map(|f| f.name.as_deref()).collect();
        assert_eq!(written, names.map(Some), "{schema:?}");
        files.extend(response.file);
    }

    let update = std::env::var_os("DCSBRIDGE_UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    for file in &files {
        let name = file.name.as_deref().expect("every file is named");
        let actual = file.content.as_deref().unwrap_or_default();
        let path = root.join("tests/golden").join(name);
        if update {
            fs::write(&path, actual).expect("the golden file writes");
            continue;
        }
        let expected = fs::read_to_string(&path).unwrap_or_default();
        if expected != actual {
            let at = first_difference(&expected, actual)
                .unwrap_or_else(|| "a line ending differs".to_owned());
            failures.push(format!("{name} differs from its golden copy at {at}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{}\n\nIf the change is meant, rewrite the copies with \
         DCSBRIDGE_UPDATE_GOLDEN=1 cargo test -p protoc-gen-dcsbridge-lua --test golden",
        failures.join("\n\n")
    );
}

#[test]
fn a_difference_names_its_line() {
    let at = first_difference("a\nb\nc\n", "a\nx\nc\n").unwrap();
    assert!(at.starts_with("line 2"), "{at}");
    assert!(
        at.contains("golden:    b") && at.contains("generated: x"),
        "{at}"
    );
    assert!(
        first_difference("a\n", "a\nb\n")
            .unwrap()
            .starts_with("line 2")
    );
    assert_eq!(first_difference("a\n", "a\n"), None);
}
