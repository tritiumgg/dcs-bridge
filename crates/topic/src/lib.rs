//! The names both sides of the wire agree on: the bridge's own topics, the
//! ones the broker knows by name and `dcsb` sends and checks, and the
//! prefix every type URL carries.
//!
//! A topic is the payload's fully-qualified type name, so each one is the
//! package and the message, and the package is the broker's: what it
//! frames, answers, consumes or addresses. The broker holds no schema, so
//! nothing here is derived from one; the test below reads the `.proto` and
//! checks that every name is declared there.
//!
//! Constants and nothing else. The broker's shipped build runs no code from
//! here, and `dcsb` shares no encoder logic with the broker through it.

#![no_std]

/// What protobuf writes in front of a fully-qualified type name to make an
/// `Any` type URL. The topic is the name, and this is what every runtime
/// expects to find before it.
pub const TYPE_URL_PREFIX: &str = "type.googleapis.com/";

/// The package every topic here is in.
pub const PACKAGE: &str = "dcsbridge.broker";

/// One constant per message, and `ALL` for the test that checks them.
///
/// `concat!` takes literals only, so the package is spelled once more
/// inside; the test holds the two together.
macro_rules! topics {
    ($($(#[$meta:meta])* $name:ident = $message:literal),* $(,)?) => {
        $(
            $(#[$meta])*
            pub const $name: &str = concat!("dcsbridge.broker.", $message);
        )*
        /// Every topic above.
        pub const ALL: &[&str] = &[$($name),*];
    };
}

topics! {
    /// Every connection's first frame.
    HANDSHAKE = "Handshake",
    /// Liveness, asked before or after authentication.
    PING = "Ping",
    /// What answers a `Ping`.
    PONG = "Pong",
    /// Authentication, the other message allowed before it.
    AUTH = "Auth",
    /// What answers an `Auth`.
    AUTH_RESULT = "AuthResult",
    /// Asks for the schema the broker serves.
    GET_SCHEMA = "GetSchema",
    /// What answers a `GetSchema`.
    SCHEMA = "Schema",
    /// A consumer's highest durably processed `seq`. Answered by nothing.
    SEQ_ACK = "SeqAck",
    /// The kill switch. Answered by nothing.
    SET_ENABLED = "SetEnabled",
    /// The acknowledgement a handler addresses to one connection, and the
    /// one topic a record may be addressed to before any registration:
    /// the bridge's own message, so the broker knows it by name. ADR 0017.
    COMMAND_ACK = "CommandAck",
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::format;

    use super::*;

    /// The macro spells the package as a literal and `PACKAGE` spells it
    /// again, so one changing without the other is a failure here.
    #[test]
    fn every_topic_is_in_the_package() {
        for topic in ALL {
            assert_eq!(
                topic.rsplit_once('.').map(|(package, _)| package),
                Some(PACKAGE),
                "{topic} is not in {PACKAGE}"
            );
        }
    }

    /// Every topic names the package the `.proto` declares and a message it
    /// holds. A constant that names a message the schema does not declare
    /// is a wire bug nothing else catches, because both sides of the wire
    /// read the same constant.
    #[test]
    fn every_topic_is_in_the_schema() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../proto/dcsbridge/broker/broker.proto"
        );
        let schema =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));

        for topic in ALL {
            let (package, message) = topic
                .rsplit_once('.')
                .expect("the topic is a qualified name");
            assert!(
                schema
                    .lines()
                    .any(|line| line.trim() == format!("package {package};")),
                "{path} does not declare package {package}"
            );
            assert!(
                schema
                    .lines()
                    .any(|line| line.trim().starts_with(&format!("message {message} "))),
                "{path} does not declare message {message}"
            );
        }
    }
}
