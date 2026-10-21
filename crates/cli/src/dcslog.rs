//! What `doctor` reads in `Logs\dcs.log`: the loader's lines from the
//! newest start.
//!
//! The loader writes under `DCSBridge`, behind a prefix DCS adds and no file
//! in the tree records, so a line is matched on that word and on the
//! loader's own message text, never on the prefix. The newest start is the
//! last banner or disabled line and every loader line after it, whether the
//! file holds one start or several. ADR 0075.

use std::io::{self, BufRead};

/// The name the loader writes under.
pub const SOURCE: &str = "DCSBridge";

/// The banner, written before anything else runs.
pub const BANNER: &str = "loading, ";

/// The one line a loader with `ENABLED = false` writes.
pub const DISABLED: &str = "disabled by ENABLED = false in ";

/// The two lines the loader writes when the hook driver stops it.
pub const STOPS: [&str; 2] = [
    "the hook driver did not load: ",
    "the hook driver did not start: ",
];

/// The newest start the log records, each line quoted from its message on.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    /// No banner and no disabled line: DCS never ran the loader.
    None,
    /// The loader is installed and off.
    Disabled(String),
    /// The banner, and the line the loader wrote if the hook driver stopped.
    Banner {
        banner: String,
        stop: Option<String>,
    },
}

/// Read `reader` to the end and return its newest start.
///
/// The file is read as bytes and decoded lossily, and a trailing carriage
/// return is dropped, since DCS writes CRLF on Windows and the banner
/// carries the user's own path, which need not be UTF-8.
pub fn newest(mut reader: impl BufRead) -> io::Result<Start> {
    let mut start = Start::None;
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            return Ok(start);
        }
        if !bytes.windows(SOURCE.len()).any(|w| w == SOURCE.as_bytes()) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let line = text.trim_end_matches('\n').trim_end_matches('\r');
        // A stop is looked for first, because the error it quotes is free
        // text and could hold the banner's words.
        if let Some(at) = STOPS.iter().find_map(|stop| line.find(stop)) {
            if let Start::Banner {
                stop: stop @ None, ..
            } = &mut start
            {
                *stop = Some(line[at..].to_owned());
            }
        } else if let Some(at) = line.find(BANNER) {
            start = Start::Banner {
                banner: line[at..].to_owned(),
                stop: None,
            };
        } else if let Some(at) = line.find(DISABLED) {
            start = Start::Disabled(line[at..].to_owned());
        }
    }
}

/// A file as the banner names it: its size and its modification time,
/// UTC to the second, or missing.
#[derive(Debug, PartialEq, Eq)]
pub enum Identity {
    Missing,
    Present { size: u64, modified: String },
}

impl std::fmt::Display for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Identity::Missing => write!(f, "missing"),
            Identity::Present { size, modified } => {
                write!(f, "{size} bytes, modified {modified}")
            }
        }
    }
}

/// The identity the banner gives the file whose write-directory path is
/// `rel`. The path is found by its own text, since the write directory
/// before it is the user's and can hold any character the separators can.
pub fn identity(banner: &str, rel: &str) -> Option<Identity> {
    let after = &banner[banner.find(rel)? + rel.len()..];
    let after = after.strip_prefix(' ')?;
    if after.starts_with("missing") {
        return Some(Identity::Missing);
    }
    let (size, after) = after.split_once(' ')?;
    let modified = after.strip_prefix("bytes, modified ")?.get(..19)?;
    Some(Identity::Present {
        size: size.parse().ok()?,
        modified: modified.to_owned(),
    })
}

/// `secs` since the Unix epoch as `YYYY-MM-DD HH:MM:SS` UTC, the way the
/// loader's `os.date('!%Y-%m-%d %H:%M:%S')` writes a modification time.
pub fn utc(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days to a civil date in the proleptic Gregorian calendar, counted in
    // 400-year eras from 0000-03-01 so the leap day falls at an era's end.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNER_LINE: &str = "2026-10-21 10:00:00.000 INFO    DCSBridge (Main): loading, \
        C:\\Users\\pilot\\Saved Games\\DCS\\Scripts/Hooks/DCSBridge.lua 1520 bytes, \
        modified 2026-10-20 09:00:00, \
        C:\\Users\\pilot\\Saved Games\\DCS\\Mods/services/DCSBridge/lua/HookDriver.lua \
        70000 bytes, modified 2026-10-20 09:00:00";

    fn read(log: &str) -> Start {
        newest(log.as_bytes()).unwrap()
    }

    fn banner() -> String {
        BANNER_LINE[BANNER_LINE.find(BANNER).unwrap()..].to_owned()
    }

    /// A banner with no stop after it is a hook driver that started, and
    /// other sources' lines, whatever they say, are passed over.
    #[test]
    fn a_banner_alone_is_a_start() {
        let log = format!(
            "2026-10-21 09:59:59.000 INFO    EDCORE (Main): loading, terrain\n\
             {BANNER_LINE}\n\
             2026-10-21 10:00:01.000 ERROR   SCRIPTING (Main): the hook driver did not start: other\n"
        );
        assert_eq!(
            read(&log),
            Start::Banner {
                banner: banner(),
                stop: None
            }
        );
    }

    /// Each stop is quoted from its message on, past whatever prefix DCS
    /// wrote before the word `DCSBridge`.
    #[test]
    fn a_stop_after_the_banner_is_quoted() {
        for stop in STOPS {
            let log =
                format!("{BANNER_LINE}\nanything at all DCSBridge {stop}no HookDriver.gen.lua\n");
            assert_eq!(
                read(&log),
                Start::Banner {
                    banner: banner(),
                    stop: Some(format!("{stop}no HookDriver.gen.lua"))
                }
            );
        }
    }

    /// The disabled line is a start of its own, and no banner is none.
    #[test]
    fn disabled_and_none_are_told_apart() {
        let log = "x INFO DCSBridge (Main): disabled by ENABLED = false in C:\\x, nothing loaded\n";
        assert_eq!(
            read(log),
            Start::Disabled("disabled by ENABLED = false in C:\\x, nothing loaded".into())
        );
        assert_eq!(read("x INFO EDCORE (Main): started\n"), Start::None);
        assert_eq!(read(""), Start::None);
    }

    /// Only the newest start counts: an older start's stop ahead of the
    /// last banner is not this start's, and a disabled start after a
    /// banner replaces it.
    #[test]
    fn only_the_newest_start_counts() {
        let log = format!(
            "{BANNER_LINE}\nx DCSBridge {}old\n{BANNER_LINE}\n",
            STOPS[1]
        );
        assert_eq!(
            read(&log),
            Start::Banner {
                banner: banner(),
                stop: None
            }
        );
        let log = format!("{BANNER_LINE}\nx DCSBridge {DISABLED}C:\\x, nothing loaded\n");
        assert!(matches!(read(&log), Start::Disabled(_)));
    }

    /// A stop whose quoted error holds the banner's words and the disabled
    /// line's stays a stop on the current banner, rather than starting a
    /// new one.
    #[test]
    fn a_stop_quoting_the_other_lines_stays_a_stop() {
        let error = format!("{}x: loading, {DISABLED}y", STOPS[0]);
        let log = format!("{BANNER_LINE}\nx ERROR DCSBridge (Main): {error}\n");
        assert_eq!(
            read(&log),
            Start::Banner {
                banner: banner(),
                stop: Some(error)
            }
        );
    }

    /// A Windows log with CRLF endings and a byte that is not UTF-8 in the
    /// user's path reads, with no carriage return left on the quote.
    #[test]
    fn crlf_and_a_byte_that_is_not_utf8_read() {
        let mut log = Vec::new();
        log.extend_from_slice(
            b"x INFO DCSBridge (Main): loading, C:\\Users\\J\xf6rg\\x 1 bytes\r\n",
        );
        log.extend_from_slice(b"x ERROR DCSBridge (Main): the hook driver did not load: boom\r\n");
        let Start::Banner { banner, stop } = newest(&log[..]).unwrap() else {
            panic!("no banner read");
        };
        assert_eq!(banner, "loading, C:\\Users\\J\u{fffd}rg\\x 1 bytes");
        assert_eq!(stop.as_deref(), Some("the hook driver did not load: boom"));
    }

    /// Each file's identity is read after its own path, present or
    /// missing, and a path the banner does not hold is none.
    #[test]
    fn the_banner_names_each_file_by_size_and_time() {
        let banner = banner();
        assert_eq!(
            identity(&banner, "Scripts/Hooks/DCSBridge.lua"),
            Some(Identity::Present {
                size: 1520,
                modified: "2026-10-20 09:00:00".into()
            })
        );
        assert_eq!(
            identity(&banner, "Mods/services/DCSBridge/lua/HookDriver.lua")
                .unwrap()
                .to_string(),
            "70000 bytes, modified 2026-10-20 09:00:00"
        );
        let missing = "loading, C:\\DCS\\Scripts/Hooks/DCSBridge.lua 9 bytes, modified \
            2026-10-20 09:00:00, C:\\DCS\\Mods/services/DCSBridge/lua/HookDriver.lua missing";
        assert_eq!(
            identity(missing, "Mods/services/DCSBridge/lua/HookDriver.lua"),
            Some(Identity::Missing)
        );
        assert_eq!(identity(&banner, "Scripts/Hooks/Other.lua"), None);
    }

    /// The formatter agrees with `date -u` at the epoch, on a leap day, on
    /// a century that is not a leap year, and at the last second of 9999.
    #[test]
    fn utc_formats_as_the_loader_does() {
        assert_eq!(utc(0), "1970-01-01 00:00:00");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(utc(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(utc(4_107_542_399), "2100-02-28 23:59:59");
        assert_eq!(utc(4_107_542_400), "2100-03-01 00:00:00");
        assert_eq!(utc(253_402_300_799), "9999-12-31 23:59:59");
    }
}
