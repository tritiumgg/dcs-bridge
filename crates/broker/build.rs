//! The import library the broker links DCS's Lua through.
//!
//! SPEC 5.1.1: *Bind to it through an import library generated from a
//! checked-in `.def` naming `lua.dll`, which needs no DCS install at build
//! time and pins exactly which Lua symbols the broker depends on.*
//!
//! `tools/mkimplib.sh` runs the same probe from a shell, for checking a
//! machine by hand. This runs it in Rust, so a Windows host with no `sh`
//! still builds.
//!
//! The `dcs-lua` feature decides whether any of this happens. It is on by
//! default because the product artifact is the Windows DLL; the host-native
//! test build turns it off and never touches the `.def`. DR-0002.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    // Without the feature this is the host-native build, which links a stock
    // Lua rather than DCS's and must leave the .def alone.
    if env::var_os("CARGO_FEATURE_DCS_LUA").is_none() {
        return;
    }

    // The feature is on by default, so every non-Windows build reaches here.
    // Nothing to do for them: the binding itself is cfg'd off too.
    if cfg_var("CARGO_CFG_TARGET_OS") != "windows" {
        return;
    }

    let def = workspace_root().join("vendor").join("lua").join("lua.def");
    println!("cargo::rerun-if-changed={}", def.display());

    let target_env = cfg_var("CARGO_CFG_TARGET_ENV");
    assert!(
        target_env == "msvc",
        "the broker links DCS's Lua through an MSVC import library, and this \
         target is x86_64-pc-windows-{target_env}. Build --target \
         x86_64-pc-windows-msvc, or turn the dcs-lua feature off for a \
         host-native build."
    );

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("lua.lib");
    generate(&def, &out);
    names_lua_dll(&out);

    println!(
        "cargo::rustc-link-search=native={}",
        out.parent().unwrap().display()
    );
    println!("cargo::rustc-link-lib=dylib=lua");
}

/// A cargo-set variable that is always present and always UTF-8.
fn cfg_var(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("cargo sets {name}"))
}

/// The checkout root, two levels above `crates/broker`.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/broker sits two levels below the checkout root")
        .to_path_buf()
}

/// Turn `def` into the import library `out`, with whichever tool is installed.
///
/// The three that can do it, in the order `tools/mkimplib.sh` tries them:
/// `llvm-dlltool` and `llvm-lib` from a full LLVM install, and MSVC's
/// `lib.exe` on a Windows host with the Build Tools.
fn generate(def: &Path, out: &Path) {
    if let Some(tool) = find_tool(&["llvm-dlltool"]) {
        run(
            Command::new(&tool)
                .arg("-m")
                .arg("i386:x86-64")
                .arg("-d")
                .arg(def)
                .arg("-l")
                .arg(out),
            &tool,
        );
        return;
    }

    if let Some(tool) = find_tool(&["llvm-lib", "lib.exe", "lib"]) {
        run(
            Command::new(&tool)
                .arg(format!("/def:{}", def.display()))
                .arg(format!("/out:{}", out.display()))
                .arg("/machine:x64"),
            &tool,
        );
        return;
    }

    panic!(
        "No import-library tool found. Install one:\n\n  \
         Debian or Ubuntu   apt-get install llvm\n  \
         macOS              brew install llvm\n  \
         Windows            the MSVC Build Tools, which provide lib.exe\n\n\
         llvm-dlltool and llvm-lib both ship with a full LLVM install. A rustup\n\
         llvm-tools component is not enough on its own."
    );
}

/// The first of `names` on `PATH`, then in the places a full LLVM install
/// lands off `PATH`.
fn find_tool(names: &[&str]) -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    let dirs = env::split_paths(&path).chain(llvm_dirs());

    for dir in dirs {
        for name in names {
            for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Where a full LLVM install lands when it is not on `PATH`: Homebrew's two
/// prefixes, and Debian's versioned directories.
fn llvm_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/opt/llvm/bin"),
        PathBuf::from("/usr/local/opt/llvm/bin"),
    ];

    if let Ok(entries) = fs::read_dir("/usr/lib") {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("llvm-") {
                dirs.push(entry.path().join("bin"));
            }
        }
    }

    dirs
}

/// Run `command`, and fail the build with its output if it fails.
fn run(command: &mut Command, tool: &Path) {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("could not run {}: {e}", tool.display()));

    assert!(
        output.status.success(),
        "{} failed: {}\n{}",
        tool.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An import library naming the wrong DLL links and then fails to load inside
/// DCS, so check the name here rather than three phases later.
fn names_lua_dll(out: &Path) {
    let bytes = fs::read(out).unwrap_or_else(|e| panic!("{} was not written: {e}", out.display()));
    let name = b"lua.dll";

    assert!(
        bytes
            .windows(name.len())
            .any(|w| w.eq_ignore_ascii_case(name)),
        "{} does not name lua.dll. Check the LIBRARY line in vendor/lua/lua.def: \
         DCS ships its Lua as bin\\lua.dll, not lua51.dll. SPEC 5.1.1.",
        out.display()
    );
}
