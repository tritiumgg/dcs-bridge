//! `dcsb doctor` as a process, against scratch install trees: a working
//! one exits 0, the same tree found through the home directory exits 0, a
//! broken one exits 1 naming each cause, and no tree to check exits 2.
//!
//! Every run sets `USERPROFILE` to a scratch directory, so the machine's
//! own Saved Games tree never takes part, on a developer's Windows machine
//! or on CI's.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use dcsbridge_broker::fanout::{Capacities, Writer};
use dcsbridge_broker::transport::Listener;

const LOADER: &str = "Scripts/Hooks/DCSBridge.lua";
const DRIVER: &str = "Mods/services/DCSBridge/lua/HookDriver.lua";
const GEN: &str = "Mods/services/DCSBridge/lua/HookDriver.gen.lua";
const BROKER: &str = "Mods/services/DCSBridge/bin/lua-dcsbridge.dll";
const SCHEMA: &str = "Mods/services/DCSBridge/schema.pb";

/// The second the loader and the hook driver are stamped with, and how the
/// loader formats it.
const STAMP: u64 = 1_700_000_000;
const STAMPED: &str = "2023-11-14 22:13:20";

/// An empty scratch directory named for the case and the process.
fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("dcsb-doctor-process-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Copy the tree at `from` into `to`.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The repository's interface version, off its hook driver's line.
fn interface() -> String {
    let driver = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../lua")
            .join(DRIVER),
    )
    .unwrap();
    driver
        .lines()
        .find_map(|line| line.strip_prefix("local INTERFACE = '")?.strip_suffix('\''))
        .unwrap()
        .to_owned()
}

/// A working install under `dir`: the repository's Lua files, a stand-in
/// for each built file with the broker's carrying the interface marker,
/// and a `dcs.log` whose banner names the loader and hook driver as they
/// are. Returns the banner line.
fn install(dir: &Path) -> String {
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lua"),
        dir,
    );
    for rel in [GEN, BROKER, SCHEMA] {
        fs::create_dir_all(dir.join(rel).parent().unwrap()).unwrap();
    }
    fs::write(dir.join(GEN), "return {}\n").unwrap();
    fs::write(dir.join(SCHEMA), b"\x0a\x00").unwrap();
    let mut dll = b"MZ\x90\0".to_vec();
    dll.extend_from_slice(format!("dcsbridge-interface={}\0", interface()).as_bytes());
    fs::write(dir.join(BROKER), dll).unwrap();

    let mut names = Vec::new();
    for rel in [LOADER, DRIVER] {
        let path = dir.join(rel);
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(STAMP))
            .unwrap();
        let size = fs::metadata(&path).unwrap().len();
        names.push(format!(
            "{}/{rel} {size} bytes, modified {STAMPED}",
            dir.display()
        ));
    }
    let banner = format!(
        "2026-10-21 10:00:00.000 INFO    DCSBridge (Main): loading, {}",
        names.join(", ")
    );
    fs::create_dir_all(dir.join("Logs")).unwrap();
    fs::write(dir.join("Logs/dcs.log"), format!("{banner}\r\n")).unwrap();
    banner
}

/// Run `dcsb doctor` with `args` and `home` as the home directory.
fn doctor(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dcsb"))
        .arg("doctor")
        .args(args)
        .env("USERPROFILE", home)
        .output()
        .unwrap()
}

/// The `fail` lines of a run's stdout.
fn fails(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("fail "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn doctor_diagnoses_an_install_in_one_command() {
    let (writer, _commit, connections) = Writer::spawn(64);
    let answers = Arc::new(dcsbridge_broker::state::Global);
    let listener =
        Listener::spawn("127.0.0.1:0", connections, Capacities::each(4), answers).unwrap();
    let live = listener.local_addr().to_string();
    let nobody = scratch("nobody");

    // A working tree named with --dir, and a bridge answering.
    let tree = scratch("tree");
    install(&tree);
    let tree_arg = tree.to_str().unwrap();
    let output = doctor(&nobody, &["--dir", tree_arg, "--addr", &live]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(fails(&output).is_empty(), "{stdout}");
    assert!(stdout.contains(&format!("ok   {live} answers")), "{stdout}");
    assert!(
        stdout.contains(&format!("ok   interface {} in", interface())),
        "{stdout}"
    );
    assert!(stdout.contains("ok   the hook driver started"), "{stdout}");

    // The same tree found under the home directory, with no --dir.
    let home = scratch("home");
    let beta = home.join("Saved Games").join("DCS.openbeta");
    install(&beta);
    let output = doctor(&home, &["--addr", &live]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    assert!(
        stdout.starts_with(&format!("note write directory {}\n", beta.display())),
        "{stdout}"
    );

    // A broken tree: the generated file gone, the start stopped on it, and
    // nothing listening. Every cause is named in the one run.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = closed.local_addr().unwrap().to_string();
    drop(closed);
    let banner = install(&tree);
    fs::remove_file(tree.join(GEN)).unwrap();
    let stop = "the hook driver did not start: no HookDriver.gen.lua";
    fs::write(
        tree.join("Logs/dcs.log"),
        format!("{banner}\r\n2026-10-21 10:00:00.100 ERROR   DCSBridge (Main): {stop}\r\n"),
    )
    .unwrap();
    let output = doctor(&nobody, &["--dir", tree_arg, "--addr", &dead]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let fails = fails(&output);
    assert_eq!(fails.len(), 3, "{stdout}");
    assert!(
        fails[0].contains("HookDriver.gen.lua is missing"),
        "{stdout}"
    );
    assert!(fails[1].ends_with(stop), "{stdout}");
    assert!(
        fails[2].contains(&format!("cannot connect to {dead}")),
        "{stdout}"
    );

    // No tree to check: nothing was learned.
    let output = doctor(&nobody, &["--addr", &dead]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(output.stdout.is_empty());
    assert!(stderr.contains("pass --dir"), "{stderr}");

    drop(listener);
    drop(writer);
    for dir in [nobody, tree, home] {
        fs::remove_dir_all(dir).unwrap();
    }
}
