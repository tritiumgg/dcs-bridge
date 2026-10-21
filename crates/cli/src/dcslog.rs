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
}
