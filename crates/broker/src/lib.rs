//! The DCS-Bridge broker.
//!
//! A stub. Task 2.1 gives it a host-native test harness, and the rings,
//! threads and framed TCP transport follow through Phase 2.

/// The broker build, carried in the handshake for bug reports.
///
/// SPEC 13.3 compares nothing against it.
pub const BROKER_VERSION: &str = env!("CARGO_PKG_VERSION");
