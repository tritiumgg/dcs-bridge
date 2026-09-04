//! The handshake: the first frame every connection receives.
//!
//! It says that a bridge is here and which build of it, so a consumer can
//! decide whether it speaks this protocol and tell a broker restart from a
//! reconnect. It says nothing about the mission, because nothing has
//! authenticated yet, and the mission is what authentication guards.
//!
//! A connection's stream starts with it at `seq` 1: the listener encodes it
//! at accept and hands it to the writer thread as the connection's first
//! answer, on the channel that attaches the connection, so it is numbered
//! before anything fanned out to it. ADR 0018.

use crate::encode::Encoder;
use crate::transport::Record;

/// The handshake's topic: the bridge's own message, in the bridge's own
/// package, known here by name.
pub const TOPIC: &[u8] = b"dcs.bridge.Handshake";

/// The bytes the handshake takes at most: the wrapper, four short fields
/// and a hash.
const BYTES: usize = 256;

/// What the handshake carries. `dcs.bridge.Handshake` in the schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Handshake {
    /// The frame format and the handshake's own shape, as a number the
    /// consumer compares. The broker states it and refuses nothing.
    pub protocol: u32,
    /// The broker build, for a bug report. Compared by nobody.
    pub broker: &'static str,
    /// Names this broker process. A consumer that reconnects and reads a
    /// different one is talking to a restarted broker, whose `seq` origin
    /// and retained set are new.
    pub instance_id: u64,
    /// The SHA-256 of the schema the broker serves, absent until the hook
    /// driver hands the schema over.
    pub schema_sha256: Option<[u8; 32]>,
}

impl Handshake {
    /// Encode as an envelope tail, the form the rings carry.
    ///
    /// The buffer is sized for every field at its largest, so a put here
    /// cannot fail; one that did would be a change to this message that
    /// forgot to change the size.
    pub fn encode(&self) -> Record {
        let mut e = Encoder::with_capacity(BYTES);
        e.begin(TOPIC);
        e.integer(1, i64::from(self.protocol))
            .expect("the handshake fits its buffer");
        e.string(2, self.broker.as_bytes())
            .expect("the handshake fits its buffer");
        // A uint64 is a varint of the same bits an int64 is, so the cast
        // changes nothing on the wire.
        e.integer(3, self.instance_id as i64)
            .expect("the handshake fits its buffer");
        if let Some(hash) = &self.schema_sha256 {
            e.string(4, hash).expect("the handshake fits its buffer");
        }
        Record::from(e.commit().expect("the handshake fits its buffer"))
    }
}

/// A number that names this process, taken once.
///
/// It only has to differ between two starts of the broker, so the random
/// keys `std` seeds its hasher with are enough, and the shipped build takes
/// no crate for it.
pub fn instance_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::hash::RandomState::new().build_hasher();
    hasher.write_u64(std::process::id().into());
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::TYPE_URL_PREFIX;
    use prost::Message;

    /// `dcs.bridge.Handshake` as a consumer decodes it.
    #[derive(Clone, PartialEq, Message)]
    struct Decoded {
        #[prost(uint32, tag = "1")]
        protocol: u32,
        #[prost(string, tag = "2")]
        broker: String,
        #[prost(uint64, tag = "3")]
        instance_id: u64,
        #[prost(bytes = "vec", optional, tag = "4")]
        schema_sha256: Option<Vec<u8>>,
    }

    fn decode(tail: &Record) -> (String, Decoded) {
        #[derive(Clone, PartialEq, Message)]
        struct Tail {
            #[prost(message, optional, tag = "4")]
            payload: Option<prost_types::Any>,
        }
        let any = Tail::decode(&tail[..])
            .expect("the tail decodes")
            .payload
            .expect("a payload");
        (
            any.type_url.clone(),
            Decoded::decode(&any.value[..]).expect("the handshake decodes"),
        )
    }

    /// A stock decoder reads every field back, a large instance id
    /// included, and an absent hash stays absent.
    #[test]
    fn a_stock_decoder_reads_the_handshake() {
        let handshake = Handshake {
            protocol: 7,
            broker: "1.2.3-test",
            instance_id: u64::MAX - 1,
            schema_sha256: None,
        };
        let (url, decoded) = decode(&handshake.encode());
        let mut want = TYPE_URL_PREFIX.to_vec();
        want.extend_from_slice(TOPIC);
        assert_eq!(url.as_bytes(), want);
        assert_eq!(decoded.protocol, 7);
        assert_eq!(decoded.broker, "1.2.3-test");
        assert_eq!(decoded.instance_id, u64::MAX - 1);
        assert_eq!(decoded.schema_sha256, None);

        let with_hash = Handshake {
            schema_sha256: Some([0xab; 32]),
            ..handshake
        };
        let (_, decoded) = decode(&with_hash.encode());
        assert_eq!(decoded.schema_sha256, Some(vec![0xab; 32]));
    }

    /// Two takes of the instance id differ: the consumer's test for a
    /// restart is that the number changed.
    #[test]
    fn instance_ids_differ_between_takes() {
        assert_ne!(instance_id(), instance_id());
    }
}
