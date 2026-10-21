//! The `doctor` verb: what an install looks like from outside DCS.
//!
//! Each check adds one line to a report, marked `ok`, `fail` or `note`, and
//! every check runs whatever an earlier one found, since a broken install
//! often has more than one cause. ADR 0075.

use std::io::{self, BufReader};
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};
use std::time::UNIX_EPOCH;
use std::{fmt, fs};

use crate::dcslog::{self, Identity, Start};

/// The files a release extracts over the write directory, each of which
/// the bridge needs to start. The paths are the write directory's, with
/// the separator Lua writes them with.
pub const SHIPPED: [&str; 7] = [
    LOADER,
    DRIVER,
    "Mods/services/DCSBridge/lua/HookDriver.gen.lua",
    "Mods/services/DCSBridge/lua/HookDriver.builtin.lua",
    "Mods/services/DCSBridge/lua/SimDriver.lua",
    BROKER,
    "Mods/services/DCSBridge/schema.pb",
];

/// The broker the hook driver opens.
pub const BROKER: &str = "Mods/services/DCSBridge/bin/lua-dcsbridge.dll";

/// What the broker's interface version follows in its DLL, as
/// `crates/lua-module` builds it. ADR 0074.
pub const MARKER: &[u8] = b"dcsbridge-interface=";

/// The line the hook driver's interface version is on, around the version.
pub const INTERFACE_LINE: (&str, &str) = ("local INTERFACE = '", "'");

/// The loader DCS runs at start. A Saved Games folder without it is one
/// the bridge was never extracted over.
pub const LOADER: &str = "Scripts/Hooks/DCSBridge.lua";

/// The hook driver the loader runs.
pub const DRIVER: &str = "Mods/services/DCSBridge/lua/HookDriver.lua";

/// The user's configuration, which no release ships.
const CONFIG: &str = "Config/DCSBridge.lua";

/// The two names DCS gives its Saved Games folder, by build.
const SAVED_GAMES: [&str; 2] = ["DCS", "DCS.openbeta"];

/// How a check came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// The check passed.
    Ok,
    /// The install is broken here.
    Fail,
    /// Worth knowing, and not a broken install.
    Note,
}

/// One line per check, in the order the checks ran.
#[derive(Default)]
pub struct Report {
    pub lines: Vec<(Mark, String)>,
}

impl Report {
    pub fn ok(&mut self, text: impl Into<String>) {
        self.lines.push((Mark::Ok, text.into()));
    }

    pub fn fail(&mut self, text: impl Into<String>) {
        self.lines.push((Mark::Fail, text.into()));
    }

    pub fn note(&mut self, text: impl Into<String>) {
        self.lines.push((Mark::Note, text.into()));
    }

    /// Whether any check failed, which is what the exit code reports.
    pub fn failed(&self) -> bool {
        self.lines.iter().any(|(mark, _)| *mark == Mark::Fail)
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (mark, text) in &self.lines {
            let mark = match mark {
                Mark::Ok => "ok",
                Mark::Fail => "fail",
                Mark::Note => "note",
            };
            writeln!(f, "{mark:<4} {text}")?;
        }
        Ok(())
    }
}

/// `rel` with the host's separator, the way a user would type it.
pub fn shown(rel: &str) -> String {
    rel.replace('/', MAIN_SEPARATOR_STR)
}

/// The write directories to check: `explicit` when given, otherwise each
/// Saved Games folder under `home` that holds the loader. An error is
/// nothing to check, which is exit 2. ADR 0073.
pub fn write_dirs(explicit: Option<&Path>, home: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    if let Some(dir) = explicit {
        if !dir.is_dir() {
            return Err(format!("no directory at {}", dir.display()));
        }
        return Ok(vec![dir.to_path_buf()]);
    }
    let Some(home) = home else {
        return Err("no home directory, since USERPROFILE is not set; \
             pass --dir with the Saved Games directory"
            .into());
    };
    let candidates: Vec<PathBuf> = SAVED_GAMES
        .iter()
        .map(|name| home.join("Saved Games").join(name))
        .collect();
    let found: Vec<PathBuf> = candidates
        .iter()
        .filter(|dir| dir.join(LOADER).is_file())
        .cloned()
        .collect();
    if found.is_empty() {
        return Err(format!(
            "no {} under {} or {}; pass --dir with the Saved Games directory the release was extracted over",
            shown(LOADER),
            candidates[0].display(),
            candidates[1].display()
        ));
    }
    Ok(found)
}

/// Check the install at `dir`, everything that reads files.
pub fn check(dir: &Path) -> Report {
    let mut report = Report::default();
    report.note(format!("write directory {}", dir.display()));
    placement(dir, &mut report);
    start(dir, &mut report);
    interfaces(dir, &mut report);
    report
}

/// One line with both interface versions, read off the installed hook
/// driver and broker: equal, unequal, or which one names none. The hook
/// driver refuses a broker whose version differs from its own. ADR 0074.
fn interfaces(dir: &Path, report: &mut Report) {
    let (driver, broker) = match (fs::read(dir.join(DRIVER)), fs::read(dir.join(BROKER))) {
        (Ok(driver), Ok(broker)) => (driver, broker),
        (Err(_), _) => {
            report.note(format!(
                "no interface versions: {} does not read",
                shown(DRIVER)
            ));
            return;
        }
        (_, Err(_)) => {
            report.note(format!(
                "no interface versions: {} does not read",
                shown(BROKER)
            ));
            return;
        }
    };
    let driver = driver_interface(&String::from_utf8_lossy(&driver));
    let broker = broker_interface(&broker);
    match (driver, broker) {
        (Some(driver), Some(broker)) if driver == broker => report.ok(format!(
            "interface {driver} in {} and in {}",
            shown(DRIVER),
            shown(BROKER)
        )),
        (Some(driver), Some(broker)) => report.fail(format!(
            "{} is interface {driver} and {} is interface {broker}: they are from different builds; extract one release's zip again",
            shown(DRIVER),
            shown(BROKER)
        )),
        (driver, broker) => {
            if driver.is_none() {
                report.fail(format!("{} names no interface version", shown(DRIVER)));
            }
            if broker.is_none() {
                report.fail(format!(
                    "{} carries no interface version: it is from an older build, or not this project's",
                    shown(BROKER)
                ));
            }
        }
    }
}

/// The hook driver's interface version: the first line of the shape
/// `local INTERFACE = '...'`.
pub fn driver_interface(text: &str) -> Option<String> {
    let (before, after) = INTERFACE_LINE;
    text.lines()
        .map(|line| line.trim_end_matches('\r'))
        .find_map(|line| line.strip_prefix(before)?.strip_suffix(after))
        .map(str::to_owned)
}

/// The broker's interface version: the printable bytes between the marker
/// and the NUL ending it, the first time the marker appears.
pub fn broker_interface(bytes: &[u8]) -> Option<String> {
    let at = bytes.windows(MARKER.len()).position(|w| w == MARKER)? + MARKER.len();
    let rest = &bytes[at..];
    let end = rest.iter().take(64).position(|&b| b == 0)?;
    let version = &rest[..end];
    if version.is_empty() || !version.iter().all(u8::is_ascii_graphic) {
        return None;
    }
    String::from_utf8(version.to_vec()).ok()
}

/// DCS's own log, where the loader writes.
const DCS_LOG: &str = "Logs/dcs.log";

/// Two lines from the newest start in `dcs.log`: the banner, then whether
/// the hook driver started. A disabled loader is one line, and no banner
/// is one line saying the loader never ran.
fn start(dir: &Path, report: &mut Report) {
    let log = shown(DCS_LOG);
    let file = match fs::File::open(dir.join(DCS_LOG)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            report.fail(format!(
                "no {log}: DCS has not started from this write directory"
            ));
            return;
        }
        Err(error) => {
            report.fail(format!("cannot read {log}: {error}"));
            return;
        }
    };
    match dcslog::newest(BufReader::new(file)) {
        Err(error) => report.fail(format!("cannot read {log}: {error}")),
        Ok(Start::None) => report.fail(format!(
            "no banner in {log}: DCS did not find the loader at its last start, or could not parse it"
        )),
        Ok(Start::Disabled(line)) => report.fail(format!("{log}: {line}")),
        Ok(Start::Banner { banner, stop }) => {
            report.ok(format!("{log}: {banner}"));
            same_build(dir, &banner, report);
            match stop {
                None => report.ok("the hook driver started"),
                Some(line) => report.fail(format!("{log}: {line}")),
            }
        }
    }
}

/// One line saying whether the loader and the hook driver are the files
/// the banner names. A difference means the install changed after DCS last
/// started, so what the log says is about other files.
fn same_build(dir: &Path, banner: &str, report: &mut Report) {
    let mut changed = Vec::new();
    for rel in [LOADER, DRIVER] {
        let Some(logged) = dcslog::identity(banner, rel) else {
            report.fail(format!("the banner does not name {}", shown(rel)));
            return;
        };
        let installed = installed(&dir.join(rel));
        if installed != logged {
            changed.push(format!(
                "{} is {installed} and the banner names it {logged}",
                shown(rel)
            ));
        }
    }
    if changed.is_empty() {
        report.ok("the banner names the installed loader and hook driver");
    } else {
        report.fail(format!(
            "{}: it changed since DCS last started",
            changed.join("; ")
        ));
    }
}

/// `path`'s identity as the loader would write it.
fn installed(path: &Path) -> Identity {
    let Ok(metadata) = fs::metadata(path) else {
        return Identity::Missing;
    };
    // A time before the epoch, or one the file system does not keep,
    // reads as the epoch, and so as a file changed since the banner.
    let secs = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    Identity::Present {
        size: metadata.len(),
        modified: dcslog::utc(secs),
    }
}

/// One line per shipped file: present, or missing by name. The
/// configuration is a note either way, since the hook driver runs on its
/// defaults without one.
fn placement(dir: &Path, report: &mut Report) {
    for rel in SHIPPED {
        if dir.join(rel).is_file() {
            report.ok(shown(rel));
        } else {
            report.fail(format!("{} is missing", shown(rel)));
        }
    }
    if dir.join(CONFIG).is_file() {
        report.note(format!("{} is present", shown(CONFIG)));
    } else {
        report.note(format!(
            "{} is absent; every key has its default",
            shown(CONFIG)
        ));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{fs, slice};

    /// An empty scratch directory under the system's temporary one, named
    /// for the test and the process so parallel tests and runs never meet.
    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dcsb-doctor-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Every shipped file under `dir`, each holding its own name.
    pub(crate) fn install(dir: &Path) {
        for rel in SHIPPED {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, rel).unwrap();
        }
    }

    /// The banner the loader would write for the loader and hook driver
    /// under `dir`, after setting both files' times to a known second.
    fn banner(dir: &Path) -> String {
        let at = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let mut names = Vec::new();
        for rel in [LOADER, DRIVER] {
            let path = dir.join(rel);
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(at).unwrap();
            let size = fs::metadata(&path).unwrap().len();
            names.push(format!(
                "{}{MAIN_SEPARATOR_STR}{rel} {size} bytes, modified 2023-11-14 22:13:20",
                dir.display()
            ));
        }
        format!("loading, {}", names.join(", "))
    }

    /// The lines one check adds for `dir`, alone.
    pub(crate) fn by(check: fn(&Path, &mut Report), dir: &Path) -> Report {
        let mut report = Report::default();
        check(dir, &mut report);
        report
    }

    pub(crate) fn marked(report: &Report, mark: Mark) -> Vec<&str> {
        report
            .lines
            .iter()
            .filter(|(m, _)| *m == mark)
            .map(|(_, text)| text.as_str())
            .collect()
    }

    /// A full tree passes every placement line, and each file taken away
    /// is named on a line of its own.
    #[test]
    fn placement_names_each_missing_file() {
        let dir = scratch("placement");
        install(&dir);
        let report = by(placement, &dir);
        assert!(!report.failed(), "{report}");
        assert_eq!(marked(&report, Mark::Ok).len(), SHIPPED.len(), "{report}");

        for rel in SHIPPED {
            fs::remove_file(dir.join(rel)).unwrap();
            let report = by(placement, &dir);
            assert_eq!(
                marked(&report, Mark::Fail),
                [format!("{} is missing", shown(rel))],
                "{report}"
            );
            fs::write(dir.join(rel), rel).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// No `dcs.log` is a failure naming it. A banner and a hook driver
    /// that started are two `ok` lines, the banner quoted; a stop after the
    /// banner fails, quoted. A log with no banner fails.
    #[test]
    fn the_newest_start_in_dcs_log_is_reported() {
        let dir = scratch("start");
        let report = by(start, &dir);
        assert_eq!(
            marked(&report, Mark::Fail),
            [format!(
                "no {}: DCS has not started from this write directory",
                shown(DCS_LOG)
            )]
        );

        install(&dir);
        let text = banner(&dir);
        let banner = format!("x INFO DCSBridge (Main): {text}\n");
        fs::create_dir_all(dir.join("Logs")).unwrap();
        fs::write(dir.join(DCS_LOG), &banner).unwrap();
        let report = by(start, &dir);
        assert!(!report.failed(), "{report}");
        assert_eq!(
            marked(&report, Mark::Ok),
            [
                format!("{}: {text}", shown(DCS_LOG)),
                "the banner names the installed loader and hook driver".into(),
                "the hook driver started".into()
            ]
        );

        let stopped = format!(
            "{banner}x ERROR DCSBridge (Main): {}boom\n",
            dcslog::STOPS[1]
        );
        fs::write(dir.join(DCS_LOG), stopped).unwrap();
        let report = by(start, &dir);
        assert_eq!(
            marked(&report, Mark::Fail),
            [format!("{}: {}boom", shown(DCS_LOG), dcslog::STOPS[1])]
        );

        let disabled = format!("{}C:\\DCS\\{LOADER}, nothing loaded", dcslog::DISABLED);
        fs::write(
            dir.join(DCS_LOG),
            format!("{banner}x INFO DCSBridge (Main): {disabled}\n"),
        )
        .unwrap();
        let report = by(start, &dir);
        assert!(report.failed(), "{report}");
        assert_eq!(
            marked(&report, Mark::Fail),
            [format!("{}: {disabled}", shown(DCS_LOG))]
        );
        assert!(marked(&report, Mark::Ok).is_empty(), "{report}");

        fs::write(dir.join(DCS_LOG), "x INFO EDCORE (Main): up\n").unwrap();
        let report = by(start, &dir);
        assert!(
            marked(&report, Mark::Fail)[0].starts_with("no banner"),
            "{report}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A hook driver replaced after DCS started is named with both sizes
    /// and times, and the loader, unchanged, is not.
    #[test]
    fn a_file_changed_since_the_banner_is_named() {
        let dir = scratch("changed");
        install(&dir);
        let banner = banner(&dir);
        fs::write(dir.join(DRIVER), "a longer hook driver").unwrap();
        let mut report = Report::default();
        same_build(&dir, &banner, &mut report);
        let fails = marked(&report, Mark::Fail);
        assert_eq!(fails.len(), 1, "{report}");
        assert!(fails[0].starts_with(&shown(DRIVER)), "{report}");
        assert!(
            fails[0].contains("20 bytes")
                && fails[0].contains("the banner names it 42 bytes, modified 2023-11-14 22:13:20"),
            "{report}"
        );
        assert!(!fails[0].contains(&shown(LOADER)), "{report}");

        let mut report = Report::default();
        same_build(&dir, "loading, nothing", &mut report);
        assert_eq!(
            marked(&report, Mark::Fail),
            [format!("the banner does not name {}", shown(LOADER))]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The configuration is a note whether it is there or not.
    #[test]
    fn the_configuration_is_a_note_either_way() {
        let dir = scratch("config");
        install(&dir);
        let absent = by(placement, &dir);
        fs::create_dir_all(dir.join("Config")).unwrap();
        fs::write(dir.join(CONFIG), "").unwrap();
        let present = by(placement, &dir);
        for report in [absent, present] {
            assert!(!report.failed(), "{report}");
            assert!(
                marked(&report, Mark::Note)
                    .iter()
                    .any(|text| text.starts_with(&shown(CONFIG))),
                "{report}"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Without `--dir` each Saved Games name holding the loader is found,
    /// both when both do, and neither is nothing to check. `--dir` wins,
    /// and naming no directory is nothing to check too.
    #[test]
    fn the_write_directory_is_found_under_home_or_named() {
        let home = scratch("home");
        let none = write_dirs(None, Some(&home)).unwrap_err();
        assert!(none.contains("pass --dir"), "{none}");
        assert!(write_dirs(None, None).unwrap_err().contains("USERPROFILE"));

        let beta = home.join("Saved Games").join("DCS.openbeta");
        fs::create_dir_all(beta.join("Scripts/Hooks")).unwrap();
        fs::write(beta.join(LOADER), "").unwrap();
        assert_eq!(
            write_dirs(None, Some(&home)).unwrap(),
            slice::from_ref(&beta)
        );

        let stable = home.join("Saved Games").join("DCS");
        fs::create_dir_all(stable.join("Scripts/Hooks")).unwrap();
        assert_eq!(
            write_dirs(None, Some(&home)).unwrap(),
            slice::from_ref(&beta),
            "a folder with no loader counted"
        );
        fs::write(stable.join(LOADER), "").unwrap();
        assert_eq!(
            write_dirs(None, Some(&home)).unwrap(),
            [stable.clone(), beta.clone()]
        );

        let named = scratch("named");
        assert_eq!(
            write_dirs(Some(&named), Some(&home)).unwrap(),
            slice::from_ref(&named)
        );
        let gone = named.join("gone");
        assert!(
            write_dirs(Some(&gone), Some(&home))
                .unwrap_err()
                .contains("no directory")
        );

        fs::remove_dir_all(&home).unwrap();
        fs::remove_dir_all(&named).unwrap();
    }

    /// The marker is found among other bytes and read to its NUL; no
    /// marker, no NUL within reach, or an empty or unprintable version is
    /// none.
    #[test]
    fn the_broker_version_is_read_off_its_marker() {
        let dll = b"MZ\x90\0junk\0dcsbridge-interface=5\0more dcsbridge-interface=6\0";
        assert_eq!(broker_interface(dll).as_deref(), Some("5"));
        assert_eq!(broker_interface(b"MZ\x90\0luaopen_dcsbridge\0"), None);
        assert_eq!(broker_interface(b"dcsbridge-interface=5"), None);
        assert_eq!(broker_interface(b"dcsbridge-interface=\0"), None);
        assert_eq!(broker_interface(b"dcsbridge-interface=5\x01\0"), None);
        assert_eq!(
            driver_interface("local x = 1\r\nlocal INTERFACE = '5'\r\n").as_deref(),
            Some("5")
        );
        assert_eq!(driver_interface("local INTERFACE = 5\n"), None);
    }

    /// Equal versions are one `ok` line naming both files; unequal ones
    /// fail naming both versions; a broker with no marker fails as an
    /// older build; a file that is not there is a note, since placement
    /// has failed already.
    #[test]
    fn both_interface_versions_are_compared() {
        let dir = scratch("interfaces");
        install(&dir);
        let write = |driver: &str, broker: &[u8]| {
            fs::write(dir.join(DRIVER), driver).unwrap();
            fs::write(dir.join(BROKER), broker).unwrap();
            by(interfaces, &dir)
        };

        let report = write("local INTERFACE = '5'\n", b"\0dcsbridge-interface=5\0");
        assert_eq!(
            marked(&report, Mark::Ok),
            [format!(
                "interface 5 in {} and in {}",
                shown(DRIVER),
                shown(BROKER)
            )]
        );

        let report = write("local INTERFACE = '4'\n", b"\0dcsbridge-interface=5\0");
        let fails = marked(&report, Mark::Fail);
        assert_eq!(fails.len(), 1, "{report}");
        assert!(
            fails[0].contains("is interface 4 and") && fails[0].contains("is interface 5:"),
            "{report}"
        );

        let report = write("local INTERFACE = '5'\n", b"MZ");
        let fails = marked(&report, Mark::Fail);
        assert_eq!(fails.len(), 1, "{report}");
        assert!(fails[0].contains("from an older build"), "{report}");

        let report = write("local x = 1\n", b"\0dcsbridge-interface=5\0");
        assert_eq!(
            marked(&report, Mark::Fail),
            [format!("{} names no interface version", shown(DRIVER))]
        );

        fs::remove_file(dir.join(DRIVER)).unwrap();
        let report = by(interfaces, &dir);
        assert!(!report.failed(), "{report}");
        assert_eq!(
            marked(&report, Mark::Note),
            [format!(
                "no interface versions: {} does not read",
                shown(DRIVER)
            )]
        );

        fs::write(dir.join(DRIVER), "local INTERFACE = '5'\n").unwrap();
        fs::remove_file(dir.join(BROKER)).unwrap();
        let report = by(interfaces, &dir);
        assert!(!report.failed(), "{report}");
        assert_eq!(marked(&report, Mark::Note).len(), 1, "{report}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
