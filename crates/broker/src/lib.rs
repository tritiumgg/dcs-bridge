//! The DCS-Bridge broker.
//!
//! Rings, threads and the framed TCP transport come with Phase 2. Nothing here
//! touches DCS, and nothing here is FFI: `crates/lua-module` carries this code
//! into a Lua state, and holds every declaration that needs a Lua symbol.
//! ADR 0005.

pub mod encode;
pub mod fanout;
pub mod ring;
pub mod state;
mod sync;

pub use state::bridge;

/// The broker build, carried in the handshake for bug reports.
///
/// Nothing compares against it. Compatibility is decided by the protocol and
/// interface versions, which move for their own reasons.
pub const BROKER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    /// `tools/stage-release.sh` reads the release version out of `Cargo.toml`
    /// and matches it against a tag with the prerelease suffix cut off, so the
    /// three numbers in front of that suffix are a shape the pipeline relies
    /// on rather than a convention.
    #[test]
    fn broker_version_is_three_numbers() {
        let core = BROKER_VERSION
            .split(['-', '+'])
            .next()
            .expect("split always yields one part");
        let parts: Vec<&str> = core.split('.').collect();

        assert_eq!(
            parts.len(),
            3,
            "{BROKER_VERSION} does not carry major.minor.patch"
        );
        for part in parts {
            assert!(
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()),
                "{BROKER_VERSION} has a non-numeric component {part:?}"
            );
        }
    }
}
