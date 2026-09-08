//! The `schema` verb: one `GetSchema` in, the served `FileDescriptorSet`
//! out, written to a file.
//!
//! The bytes are the ones the hook driver read from the deployed
//! `schema.pb` and handed the broker once, so a file written here that
//! hashes to the deployed one shows the hand-off is whole. The handshake
//! carries the SHA-256 of what the broker serves, and the set is checked
//! against it before it is written: a consumer that decodes records by this
//! set is owed the one the bridge means. `GetSchema` needs a session, so a
//! token goes first.

use std::io::{self, Read};

use prost::Message;
use sha2::{Digest, Sha256};

use crate::wire::{self, AuthResult, handshake_sha256, read_frame};
use dcsbridge_topic as topic;

/// `dcsbridge.broker.Schema`, as `proto/dcsbridge/broker/broker.proto` numbers
/// it: the set, or why there is none. Exactly one is set.
#[derive(Clone, PartialEq, Message)]
pub struct Schema {
    /// The compiled `FileDescriptorSet`, as handed to the broker.
    #[prost(bytes = "vec", optional, tag = "1")]
    pub file_descriptor_set: Option<Vec<u8>>,
    /// Why there is no set to serve.
    #[prost(string, optional, tag = "2")]
    pub error: Option<String>,
}

/// What a run learned, once it stopped reading.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The set, and its SHA-256, which the handshake agreed with or did
    /// not carry.
    Fetched { set: Vec<u8>, sha256: [u8; 32] },
    /// The set hashes to something other than what the handshake said, so
    /// it is not the schema the bridge means to serve.
    Mismatch,
    /// The bridge answered that it holds no schema, in these words.
    NoSchema(String),
    /// The bridge refused the token, with this `AuthError` number.
    Refused(i32),
    /// The stream ended before an answer.
    Closed,
}

/// The second frame `schema` sends, after the token: an empty `GetSchema`,
/// numbered 2.
pub fn get_schema_frame() -> Vec<u8> {
    wire::frame(2, topic::GET_SCHEMA, Vec::new())
}

/// Read frames from `reader` until the token is refused or a `Schema`
/// arrives.
///
/// The handshake arrives first, unasked, and its hash is kept for the
/// check. An `AuthResult` or a `Schema` whose bytes do not decode is an
/// error rather than an outcome: nothing was learned, where `tail` reads
/// the first as a refusal because it goes on printing either way. A
/// `Schema` with neither field is a broker that broke its own contract and
/// reads as an empty error.
pub fn run(mut reader: impl Read) -> io::Result<Outcome> {
    let mut expected: Option<Vec<u8>> = None;
    while let Some(envelope) = read_frame(&mut reader)? {
        if let Some(hash) = handshake_sha256(&envelope) {
            expected = Some(hash);
            continue;
        }
        let invalid = |error| io::Error::new(io::ErrorKind::InvalidData, error);
        let (Some(topic), Some(any)) = (envelope.topic(), &envelope.payload) else {
            continue;
        };
        if topic == topic::AUTH_RESULT {
            let result = AuthResult::decode(&any.value[..]).map_err(invalid)?;
            if !result.ok {
                return Ok(Outcome::Refused(result.error));
            }
            continue;
        }
        if topic != topic::SCHEMA {
            continue;
        }
        let schema = Schema::decode(&any.value[..]).map_err(invalid)?;
        let Some(set) = schema.file_descriptor_set else {
            return Ok(Outcome::NoSchema(schema.error.unwrap_or_default()));
        };
        let sha256: [u8; 32] = Sha256::digest(&set).into();
        if expected.is_some_and(|hash| hash != sha256) {
            return Ok(Outcome::Mismatch);
        }
        return Ok(Outcome::Fetched { set, sha256 });
    }
    Ok(Outcome::Closed)
}

/// `hash` as lowercase hex, the way `sha256sum` and `Get-FileHash` print it.
pub fn hex(hash: &[u8]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{AuthResult, Handshake, auth_frame};

    /// The handshake carrying `hash`, numbered 1.
    fn handshake(hash: Option<&[u8]>) -> Vec<u8> {
        wire::frame(
            1,
            topic::HANDSHAKE,
            Handshake {
                schema_sha256: hash.map(<[u8]>::to_vec),
            }
            .encode_to_vec(),
        )
    }

    /// The token's answer, numbered 2.
    fn auth_result(ok: bool, error: i32) -> Vec<u8> {
        wire::frame(
            2,
            topic::AUTH_RESULT,
            AuthResult { ok, error }.encode_to_vec(),
        )
    }

    /// A `Schema` frame numbered 3, as the bridge answers.
    fn schema(schema: Schema) -> Vec<u8> {
        wire::frame(3, topic::SCHEMA, schema.encode_to_vec())
    }

    /// Bytes that look like a set, and are not parsed by anything here.
    fn set() -> Vec<u8> {
        (0..4096u32).map(|n| (n % 251) as u8).collect()
    }

    /// The set comes back whole when the handshake's hash agrees with it,
    /// and when the handshake carries none; a handshake that disagrees is
    /// a mismatch and the set is not returned.
    #[test]
    fn the_set_is_fetched_when_it_hashes_to_what_the_handshake_said() {
        let set = set();
        let sha256: [u8; 32] = Sha256::digest(&set).into();
        let answer = schema(Schema {
            file_descriptor_set: Some(set.clone()),
            error: None,
        });

        let mut stream = handshake(Some(&sha256));
        stream.extend(auth_result(true, 0));
        stream.extend(&answer);
        assert_eq!(
            run(&stream[..]).unwrap(),
            Outcome::Fetched {
                set: set.clone(),
                sha256
            }
        );

        let mut stream = handshake(None);
        stream.extend(auth_result(true, 0));
        stream.extend(&answer);
        assert_eq!(run(&stream[..]).unwrap(), Outcome::Fetched { set, sha256 });

        let mut other = sha256;
        other[0] ^= 0xff;
        let mut stream = handshake(Some(&other));
        stream.extend(auth_result(true, 0));
        stream.extend(&answer);
        assert_eq!(run(&stream[..]).unwrap(), Outcome::Mismatch);
    }

    /// The bridge's error comes back in its words, a refused token stops
    /// the read with the error number, a stream that ends first is no
    /// answer, and an `AuthResult` or a `Schema` whose bytes do not decode
    /// is an error rather than a verdict.
    #[test]
    fn no_schema_a_refusal_and_a_cut_stream_each_read_as_what_they_are() {
        let mut stream = handshake(Some(&[0u8; 32]));
        stream.extend(auth_result(true, 0));
        stream.extend(schema(Schema {
            file_descriptor_set: None,
            error: Some("no schema has been handed to the broker".into()),
        }));
        assert_eq!(
            run(&stream[..]).unwrap(),
            Outcome::NoSchema("no schema has been handed to the broker".into())
        );

        let mut stream = handshake(None);
        stream.extend(auth_result(false, 1));
        assert_eq!(run(&stream[..]).unwrap(), Outcome::Refused(1));

        let stream = handshake(None);
        assert_eq!(run(&stream[..]).unwrap(), Outcome::Closed);

        let mut stream = auth_result(true, 0);
        stream.extend(wire::frame(3, topic::SCHEMA, vec![0xff, 0xff, 0xff]));
        let error = run(&stream[..]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let stream = wire::frame(2, topic::AUTH_RESULT, vec![0xff, 0xff, 0xff]);
        let error = run(&stream[..]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    /// The frame `schema` sends after the token is an empty `GetSchema`
    /// numbered 2, and the hash prints as the hash tools print it.
    #[test]
    fn the_request_is_an_empty_get_schema_numbered_two() {
        let frame = get_schema_frame();
        let envelope = read_frame(&mut &frame[..]).unwrap().unwrap();
        assert_eq!(envelope.seq, 2);
        assert_eq!(envelope.topic(), Some(topic::GET_SCHEMA));
        assert!(envelope.payload.unwrap().value.is_empty());

        assert_eq!(hex(&[0xba, 0x00, 0x0f]), "ba000f");
    }

    /// Against a bridge holding a schema, the token and the request go
    /// out together before anything is read, the set comes back byte for
    /// byte with the hash the handshake carried, a wrong token is refused,
    /// and nothing reached the commit ring: the answer rides the reader
    /// thread.
    #[test]
    fn a_live_bridge_serves_the_set_it_holds_to_a_session() {
        use dcsbridge_broker::fanout::Writer;
        use dcsbridge_broker::inbound::{Answers, AuthError, Liveness, Session};
        use dcsbridge_broker::transport::{Listener, Record};
        use std::io::Write;
        use std::net::TcpStream;
        use std::sync::Arc;
        use std::time::Duration;

        /// A bridge after the hook driver's hand-off, with one token.
        struct Held {
            set: Record,
        }
        impl Answers for Held {
            fn handshake(&self) -> Record {
                dcsbridge_broker::handshake::Handshake {
                    protocol: dcsbridge_broker::PROTOCOL_VERSION,
                    broker: dcsbridge_broker::BROKER_VERSION,
                    instance_id: 42,
                    schema_sha256: Some(Sha256::digest(&self.set).into()),
                }
                .encode()
            }
            fn liveness(&self) -> Liveness {
                Liveness {
                    last_heard_ms: None,
                    alive: false,
                    enabled: true,
                }
            }
            fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
                if secret == b"schema-secret" {
                    Ok(Session {
                        token_id: "schema".into(),
                        caps: [dcsbridge_broker::registry::Capability::Read]
                            .into_iter()
                            .collect(),
                    })
                } else {
                    Err(AuthError::BadToken)
                }
            }
            fn disconnected(&self, _: &Session) {}
            fn schema(&self) -> Option<Record> {
                Some(Arc::clone(&self.set))
            }
            fn seq_ack(&self, _: u64) {}
            fn set_enabled(&self, _: bool) {}
            fn refused_no_capability(&self, _: &str) {}
        }

        // The compiled set when the schema task has written one, so the
        // test moves the deployed bytes where it can.
        let set = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/schema.pb"
        ))
        .unwrap_or_else(|_| self::set());
        let held = Arc::new(Held {
            set: Record::from(&set[..]),
        });

        let (writer, commit, connections) = Writer::spawn(64);
        let listener = Listener::spawn("127.0.0.1:0", connections, 4, held).unwrap();
        let connect = |secret: &str| {
            let mut client =
                TcpStream::connect(listener.local_addr()).expect("the listener accepts");
            let mut request = auth_frame(secret);
            request.extend(get_schema_frame());
            client.write_all(&request).expect("the request is sent");
            wire::Deadline::new(client, Duration::from_secs(30))
        };

        let outcome = run(connect("schema-secret")).unwrap();
        let Outcome::Fetched {
            set: served,
            sha256,
        } = outcome
        else {
            panic!("the set was not fetched: {outcome:?}");
        };
        assert_eq!(served, set, "the served set is not the one held");
        assert_eq!(&sha256[..], &Sha256::digest(&set)[..]);

        assert_eq!(run(connect("wrong")).unwrap(), Outcome::Refused(1));

        assert!(commit.is_empty(), "an answer went through the commit ring");
        drop(listener);
        drop(writer);
    }
}
