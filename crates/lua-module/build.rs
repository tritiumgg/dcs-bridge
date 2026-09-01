//! How the module's Lua symbols get resolved, which differs per host.
//!
//! SPEC 5.1.1: *Bind to it through an import library generated from a
//! checked-in `.def` naming `lua.dll`, which needs no DCS install at build
//! time and pins exactly which Lua symbols the broker depends on.*
//!
//! On Windows that import library is the whole story, and the `dcs-lua`
//! feature decides whether it is built. Everywhere else the symbols are left
//! undefined and resolved from the interpreter that opens the module, which is
//! what makes the host-native build loadable by a stock Lua 5.1. macOS needs
//! that stated per symbol; Linux permits it by default. ADR 0002, ADR 0006.
//!
//! `tools/mkimplib.sh` runs the same import-library probe from a shell, for
//! checking a machine by hand. This runs it in Rust, so a Windows host with no
//! `sh` still builds.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let def = workspace_root().join("vendor").join("lua").join("lua.def");
    println!("cargo::rerun-if-changed={}", def.display());

    match cfg_var("CARGO_CFG_TARGET_OS").as_str() {
        // The linker resolves nothing lazily here, so the module names its
        // Lua through an import library or not at all. Without the feature
        // this is a host-native build on a Windows host, and there is no
        // stock Lua to bind: the crate's Lua surface is cfg'd out to match.
        "windows" => {
            if env::var_os("CARGO_FEATURE_DCS_LUA").is_some() {
                import_library(&def);
            }
        }

        // Mach-O resolves an undefined symbol at load only where the link was
        // told to expect it, so name each one. Reading them out of the same
        // `.def` keeps one pinned list rather than two, and keeps a symbol
        // outside the 114 a link error rather than a crash inside DCS.
        "macos" => allow_undefined(&def),

        // ELF permits an undefined symbol in a shared object by default, and
        // the interpreter exports its own: Lua's `linux` target links with
        // `-Wl,-E`.
        _ => {}
    }
}

/// A cargo-set variable that is always present and always UTF-8.
fn cfg_var(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("cargo sets {name}"))
}

/// The checkout root, two levels above `crates/lua-module`.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/lua-module sits two levels below the checkout root")
        .to_path_buf()
}

/// Tell the Mach-O linker that each name `def` exports may be undefined.
///
/// This is the per-symbol spelling of `-undefined dynamic_lookup`. The blanket
/// form is deprecated on current `ld64` and admits every undefined symbol,
/// including a misspelled one, which then fails at load instead of at link.
fn allow_undefined(def: &Path) {
    for symbol in symbols(def) {
        // Mach-O prefixes a C symbol with an underscore.
        println!("cargo::rustc-cdylib-link-arg=-Wl,-U,_{symbol}");
    }
}

/// Every name in `def`'s `EXPORTS` list.
///
/// The format is a module-definition file: `;` comments, a `LIBRARY` line, an
/// `EXPORTS` line, then one bare name per line.
fn symbols(def: &Path) -> Vec<String> {
    let text =
        fs::read_to_string(def).unwrap_or_else(|e| panic!("could not read {}: {e}", def.display()));

    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(';'))
        .filter(|line| {
            let head = line.split_whitespace().next().unwrap_or_default();
            !head.eq_ignore_ascii_case("LIBRARY") && !head.eq_ignore_ascii_case("EXPORTS")
        })
        .map(str::to_owned)
        .collect();

    assert!(
        !names.is_empty(),
        "{} lists no symbols. SPEC 5.1.1 pins the 114 the broker may name.",
        def.display()
    );

    names
}

/// Generate the import library the broker links DCS's Lua through, and tell
/// cargo to link it.
fn import_library(def: &Path) {
    // MSVC is the only Windows environment the broker builds for, because
    // DCS's lua.dll is an MSVC build and a second toolchain would put a second
    // C runtime in the process. ADR 0003.
    let target_env = cfg_var("CARGO_CFG_TARGET_ENV");
    assert!(
        target_env == "msvc",
        "the broker links DCS's Lua through an MSVC import library, and this \
         target is x86_64-pc-windows-{target_env}. Build --target \
         x86_64-pc-windows-msvc, or turn the dcs-lua feature off for a \
         host-native build. ADR 0003 says why there is no second Windows target."
    );

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("lua.lib");
    generate(def, &out);
    names_lua_dll(&out);

    println!(
        "cargo::rustc-link-search=native={}",
        out.parent().unwrap().display()
    );
    println!("cargo::rustc-link-lib=dylib=lua");
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
