//! The `doctor` verb: what an install looks like from outside DCS.
//!
//! Each check adds one line to a report, marked `ok`, `fail` or `note`, and
//! every check runs whatever an earlier one found, since a broken install
//! often has more than one cause. ADR 0075.

use std::fmt;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

/// The files a release extracts over the write directory, each of which
/// the bridge needs to start. The paths are the write directory's, with
/// the separator Lua writes them with.
pub const SHIPPED: [&str; 7] = [
    LOADER,
    DRIVER,
    "Mods/services/DCSBridge/lua/HookDriver.gen.lua",
    "Mods/services/DCSBridge/lua/HookDriver.builtin.lua",
    "Mods/services/DCSBridge/lua/SimDriver.lua",
    "Mods/services/DCSBridge/bin/lua-dcsbridge.dll",
    "Mods/services/DCSBridge/schema.pb",
];

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
    report
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

    fn marked(report: &Report, mark: Mark) -> Vec<&str> {
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
        let report = check(&dir);
        assert!(!report.failed(), "{report}");
        assert_eq!(marked(&report, Mark::Ok).len(), SHIPPED.len(), "{report}");

        for rel in SHIPPED {
            fs::remove_file(dir.join(rel)).unwrap();
            let report = check(&dir);
            assert_eq!(
                marked(&report, Mark::Fail),
                [format!("{} is missing", shown(rel))],
                "{report}"
            );
            fs::write(dir.join(rel), rel).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The configuration is a note whether it is there or not.
    #[test]
    fn the_configuration_is_a_note_either_way() {
        let dir = scratch("config");
        install(&dir);
        let absent = check(&dir);
        fs::create_dir_all(dir.join("Config")).unwrap();
        fs::write(dir.join(CONFIG), "").unwrap();
        let present = check(&dir);
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
}
