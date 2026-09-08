//! What the two registrars share: the topic maps, and the enums their
//! values are drawn from.
//!
//! The hook driver registers from one Lua state and the sim driver from the
//! other, so the maps are one set held by [`crate::state::Bridge`], and each
//! registration merges into what the other left. ADR 0007, ADR 0017.

use std::collections::{HashMap, HashSet};

/// A topic: the fully-qualified protobuf type name of a record's payload.
///
/// Package names partition the topic space, so the name is the identity and
/// the broker needs no registry to tell two adopters apart.
pub type Topic = String;

/// The drop policy the broker applies to a record under pressure.
///
/// Mirrors `dcsbridge.broker.RecordClass` in `proto/dcsbridge/broker/broker.proto`, whose
/// numbers cross the wire. The schema's `UNSPECIFIED` member has no counterpart
/// here, because a topic with no class is refused rather than defaulted: there
/// is nothing for the broker to hold in its place.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RecordClass {
    /// Survives pressure until the ring is full of it.
    Durable = 1,
    /// The first evicted when the ring runs out of room.
    Lossy = 2,
    /// Inbound, carrying something the receiving state is asked to do.
    Command = 3,
    /// Retained and replayed, and never evicted to make room.
    Lifecycle = 4,
}

/// The Lua state a record routes to.
///
/// Mirrors `dcsbridge.broker.Target`. The schema's `UNSPECIFIED` member is resolved
/// by the generator, which writes an unspecified target into the sim driver
/// route set, so the broker is never handed one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Target {
    /// Injected into the `"server"` state, and reloaded with the mission.
    SimDriver = 1,
    /// Loaded once at DCS start, and outlives every mission.
    HookDriver = 2,
}

/// The permission a connection needs before the broker accepts a message.
///
/// Mirrors `dcsbridge.broker.Capability`. That enum is extensible and partitions its
/// numbers — the bridge takes 1 to 49 — so a built-in set or an adopter adds
/// members without touching these three.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    /// Receive records.
    Read = 1,
    /// Send a command.
    Command = 2,
    /// Reload configuration.
    Reload = 3,
}

/// The maps the two registrars share.
///
/// They cover different topic sets on purpose. `routes` carries inbound topics
/// only, because routing is what an inbound record needs, while `classes` and
/// `caps` carry every topic that crosses in either direction. So an
/// outbound-only topic has a class and a capability and no route, and that is
/// complete rather than missing something. `replies` is the topics a record
/// may be addressed to one connection on: the typed replies the schema names
/// in a request's `reply_to`, which the acknowledgement joins by name. ADR
/// 0017.
///
/// Empty until a registrar fills it, and there is no way to fill it yet: the
/// merge arrives with `shim.classes`, `shim.routes`, `shim.caps` and the
/// reply table beside them.
#[derive(Debug, Default)]
pub struct Registry {
    classes: HashMap<Topic, RecordClass>,
    routes: HashMap<Topic, Target>,
    caps: HashMap<Topic, Capability>,
    replies: HashSet<Topic>,
}

impl Registry {
    /// Whether a record on `topic` may be addressed to one connection.
    ///
    /// True for the acknowledgement and for a registered typed reply, and
    /// for nothing else: everything else fans out, and a record that reached
    /// one consumer instead of all of them would present as missing data at
    /// every other, which is why the broker refuses rather than trusts.
    pub fn is_addressable(&self, topic: &[u8]) -> bool {
        // The broker holds no schema, so which topics are replies reaches
        // it by registration, and the registration does not exist yet. The
        // acknowledgement is the bridge's own message, so the broker knows
        // it by name. ADR 0017.
        //
        // A topic is a type name, so a topic that is not UTF-8 is registered
        // nowhere and the lookup can say so without a copy.
        topic == dcsbridge_topic::COMMAND_ACK.as_bytes()
            || std::str::from_utf8(topic).is_ok_and(|topic| self.replies.contains(topic))
    }
}

impl Registry {
    /// Every registered topic's drop policy, inbound and outbound.
    pub fn classes(&self) -> &HashMap<Topic, RecordClass> {
        &self.classes
    }

    /// Every registered inbound topic's destination state.
    pub fn routes(&self) -> &HashMap<Topic, Target> {
        &self.routes
    }

    /// Every registered topic's required capability, inbound and outbound.
    pub fn caps(&self) -> &HashMap<Topic, Capability> {
        &self.caps
    }
}
