//! What the two registrars share: the topic maps, the enums their values
//! are drawn from, and the merge each registration goes through.
//!
//! The hook driver registers from one Lua state and the sim driver from the
//! other, so the maps are one set held by [`crate::state::Bridge`], and each
//! registration merges into what the other left. ADR 0007, ADR 0017.
//!
//! A merge is additive over disjoint topic sets. A row naming a registered
//! topic with the value it holds is a no-op, so a sim driver reload
//! re-registers its own tables and succeeds; a row naming one with a
//! different value refuses the whole call, and no row of that call is
//! applied. Nothing replaces or removes an entry: retiring a topic is a DCS
//! restart.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// A topic: the fully-qualified protobuf type name of a record's payload.
///
/// Package names partition the topic space, so the name is the identity and
/// the broker needs no registry to tell two adopters apart.
pub type Topic = String;

/// A value a registration table holds: one member of a mirrored enum, with
/// the name a registrar spells it by and the number the schema gives it.
pub trait Member: Copy + Eq + fmt::Debug + 'static {
    /// Every member, in schema order.
    const ALL: &'static [Self];

    /// The lowercase name a registrar may spell this member by, and the
    /// one a refusal names it by.
    fn name(self) -> &'static str;

    /// The schema's number for this member.
    fn number(self) -> u32;

    /// The member spelled `name`, exactly.
    fn from_name(name: &[u8]) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|m| m.name().as_bytes() == name)
    }

    /// The member the schema numbers `number`.
    fn from_number(number: u32) -> Option<Self> {
        Self::ALL.iter().copied().find(|m| m.number() == number)
    }
}

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

impl Member for RecordClass {
    const ALL: &'static [Self] = &[Self::Durable, Self::Lossy, Self::Command, Self::Lifecycle];

    fn name(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::Lossy => "lossy",
            Self::Command => "command",
            Self::Lifecycle => "lifecycle",
        }
    }

    fn number(self) -> u32 {
        self as u32
    }
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

impl Member for Target {
    const ALL: &'static [Self] = &[Self::SimDriver, Self::HookDriver];

    fn name(self) -> &'static str {
        match self {
            Self::SimDriver => "sim_driver",
            Self::HookDriver => "hook_driver",
        }
    }

    fn number(self) -> u32 {
        self as u32
    }
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

impl Member for Capability {
    const ALL: &'static [Self] = &[Self::Read, Self::Command, Self::Reload];

    fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Command => "command",
            Self::Reload => "reload",
        }
    }

    fn number(self) -> u32 {
        self as u32
    }
}

/// Why a registration was refused, with none of it applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conflict {
    /// The topic the call named with a value other than the one held.
    pub topic: Topic,
    /// What the topic is registered as.
    pub held: &'static str,
    /// What the call offered instead.
    pub offered: &'static str,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is registered as {}, not {}",
            self.topic, self.held, self.offered
        )
    }
}

impl std::error::Error for Conflict {}

/// Merge `rows` into `map`: every row new to the map is added, every row
/// repeating the map is passed over, and the first row disagreeing with the
/// map, or with an earlier row of the same call, refuses the call with
/// nothing added. Returns how many rows were added.
///
/// Two passes, because the refusal has to be whole: the rows are checked
/// against the map and against each other first, and reach the map only
/// once none of them conflicts.
fn merge<V: Member>(
    map: &mut HashMap<Topic, V>,
    rows: impl IntoIterator<Item = (Topic, V)>,
) -> Result<usize, Conflict> {
    let mut fresh: HashMap<Topic, V> = HashMap::new();
    for (topic, offered) in rows {
        match map.get(&topic).or_else(|| fresh.get(&topic)) {
            Some(held) if *held != offered => {
                return Err(Conflict {
                    held: held.name(),
                    offered: offered.name(),
                    topic,
                });
            }
            Some(_) => {}
            None => {
                fresh.insert(topic, offered);
            }
        }
    }
    let added = fresh.len();
    map.extend(fresh);
    Ok(added)
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
/// Each table registers through its own merge, so the tables of one
/// registrar arrive in whatever order its calls come, and a topic is
/// complete once its class and its capability have both arrived.
#[derive(Debug, Default)]
pub struct Registry {
    classes: HashMap<Topic, RecordClass>,
    routes: HashMap<Topic, Target>,
    caps: HashMap<Topic, Capability>,
    replies: HashSet<Topic>,
}

impl Registry {
    /// Merge a table of drop policies. See [`merge`].
    pub fn register_classes(
        &mut self,
        rows: impl IntoIterator<Item = (Topic, RecordClass)>,
    ) -> Result<usize, Conflict> {
        merge(&mut self.classes, rows)
    }

    /// Merge a table of destination states. See [`merge`].
    pub fn register_routes(
        &mut self,
        rows: impl IntoIterator<Item = (Topic, Target)>,
    ) -> Result<usize, Conflict> {
        merge(&mut self.routes, rows)
    }

    /// Merge a table of required capabilities. See [`merge`].
    pub fn register_caps(
        &mut self,
        rows: impl IntoIterator<Item = (Topic, Capability)>,
    ) -> Result<usize, Conflict> {
        merge(&mut self.caps, rows)
    }

    /// Add topics to the addressable set, returning how many were new.
    ///
    /// A set has no value to disagree on, so this cannot be refused: a
    /// topic is a reply or it is not, and registering it twice says the
    /// same thing twice.
    pub fn register_replies(&mut self, topics: impl IntoIterator<Item = Topic>) -> usize {
        topics
            .into_iter()
            .filter(|topic| self.replies.insert(topic.clone()))
            .count()
    }

    /// Whether a record on `topic` may be addressed to one connection.
    ///
    /// True for the acknowledgement and for a registered typed reply, and
    /// for nothing else: everything else fans out, and a record that reached
    /// one consumer instead of all of them would present as missing data at
    /// every other, which is why the broker refuses rather than trusts.
    pub fn is_addressable(&self, topic: &[u8]) -> bool {
        // The broker holds no schema, so which topics are replies reaches
        // it by registration. The acknowledgement is the bridge's own
        // message, so the broker knows it by name. ADR 0017.
        //
        // A topic is a type name, so a topic that is not UTF-8 is registered
        // nowhere and the lookup can say so without a copy.
        topic == dcsbridge_topic::COMMAND_ACK.as_bytes()
            || std::str::from_utf8(topic).is_ok_and(|topic| self.replies.contains(topic))
    }

    /// Whether `topic` has both a class and a capability, which is what a
    /// record needs before the broker opens one on it.
    ///
    /// The broker holds the schema opaque, so it can recover neither value:
    /// without a class it has no drop policy for the record, and without a
    /// capability no answer to whether a token's set covers it. A missing
    /// capability that failed open would disclose the record to every
    /// connection, so both fail closed, at the cost of the record. A route
    /// is not asked for: an outbound-only topic has none.
    ///
    /// The acknowledgement is complete by name. It is the bridge's own
    /// message, `DURABLE` in the schema, and it goes to the one connection
    /// that sent the command it answers, so its capability is that command's
    /// and no table needs to carry it. ADR 0017.
    pub fn is_complete(&self, topic: &[u8]) -> bool {
        topic == dcsbridge_topic::COMMAND_ACK.as_bytes()
            || std::str::from_utf8(topic).is_ok_and(|topic| {
                self.classes.contains_key(topic) && self.caps.contains_key(topic)
            })
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

    /// Every registered typed reply. The acknowledgement is not among them:
    /// it is addressable by name.
    pub fn replies(&self) -> &HashSet<Topic> {
        &self.replies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENT: &str = "dcsbridge.builtin.sim.UnitDestroyed";
    const COMMAND: &str = "dcsbridge.builtin.sim.SetFlag";
    const REPLY: &str = "dcsbridge.builtin.sim.FlagValue";

    fn rows<V: Member>(rows: &[(&str, V)]) -> Vec<(Topic, V)> {
        rows.iter().map(|(t, v)| (t.to_string(), *v)).collect()
    }

    /// Two registrars over disjoint topic sets both merge, and the map
    /// holds the union.
    #[test]
    fn a_second_registrar_over_new_topics_merges() {
        let mut registry = Registry::default();

        let hook = registry.register_classes(rows(&[(EVENT, RecordClass::Durable)]));
        let sim = registry.register_classes(rows(&[(COMMAND, RecordClass::Command)]));

        assert_eq!(hook, Ok(1));
        assert_eq!(sim, Ok(1));
        assert_eq!(registry.classes().len(), 2);
        assert_eq!(registry.classes()[COMMAND], RecordClass::Command);
    }

    /// A sim driver reload re-registers its own tables: the identical rows
    /// are a no-op that succeeds, and adds nothing.
    #[test]
    fn an_identical_registration_is_a_no_op() {
        let mut registry = Registry::default();
        let table = rows(&[(EVENT, Capability::Read), (COMMAND, Capability::Command)]);

        assert_eq!(registry.register_caps(table.clone()), Ok(2));
        assert_eq!(registry.register_caps(table), Ok(0));
        assert_eq!(registry.caps().len(), 2);
    }

    /// A conflicting row refuses the whole call: the rows before it are not
    /// applied either, and the refusal names the topic and both values.
    #[test]
    fn a_conflicting_row_refuses_the_whole_call() {
        let mut registry = Registry::default();
        registry
            .register_routes(rows(&[(COMMAND, Target::SimDriver)]))
            .expect("the first registration");

        let refused = registry.register_routes(rows(&[
            ("dcsbridge.builtin.sim.Other", Target::HookDriver),
            (COMMAND, Target::HookDriver),
            ("dcsbridge.builtin.sim.Another", Target::HookDriver),
        ]));

        assert_eq!(
            refused,
            Err(Conflict {
                topic: COMMAND.into(),
                held: "sim_driver",
                offered: "hook_driver",
            })
        );
        assert_eq!(
            refused.unwrap_err().to_string(),
            "dcsbridge.builtin.sim.SetFlag is registered as sim_driver, not hook_driver"
        );
        assert_eq!(registry.routes().len(), 1, "a refused call applied a row");
    }

    /// One call naming a topic twice with two values disagrees with itself,
    /// and is refused the same way.
    #[test]
    fn a_call_that_disagrees_with_itself_is_refused() {
        let mut registry = Registry::default();

        let refused = registry.register_classes(rows(&[
            (EVENT, RecordClass::Durable),
            (EVENT, RecordClass::Lossy),
        ]));

        assert_eq!(
            refused.map_err(|c| (c.held, c.offered)),
            Err(("durable", "lossy"))
        );
        assert!(
            registry.classes().is_empty(),
            "a refused call applied a row"
        );
    }

    /// An outbound-only topic registers with a class and a capability and no
    /// route, and is complete. A topic with one of the two is not, and a
    /// route adds nothing to completeness.
    #[test]
    fn a_topic_is_complete_with_a_class_and_a_capability_and_no_route() {
        let mut registry = Registry::default();
        registry
            .register_classes(rows(&[
                (EVENT, RecordClass::Durable),
                (COMMAND, RecordClass::Command),
            ]))
            .expect("classes");
        registry
            .register_caps(rows(&[(EVENT, Capability::Read)]))
            .expect("caps");
        registry
            .register_routes(rows(&[(COMMAND, Target::SimDriver)]))
            .expect("routes");

        assert!(registry.is_complete(EVENT.as_bytes()));
        assert!(
            !registry.is_complete(COMMAND.as_bytes()),
            "a class and a route made a topic complete without a capability"
        );
        assert!(!registry.is_complete(b"dcsbridge.builtin.sim.Unregistered"));
        assert!(!registry.is_complete(b"\xff"));
    }

    /// The acknowledgement is complete and addressable with nothing
    /// registered.
    #[test]
    fn the_acknowledgement_is_complete_and_addressable_by_name() {
        let registry = Registry::default();
        let ack = dcsbridge_topic::COMMAND_ACK.as_bytes();

        assert!(registry.is_complete(ack));
        assert!(registry.is_addressable(ack));
        assert!(registry.replies().is_empty());
    }

    /// A registered reply becomes addressable; a fan-out topic with a class
    /// and a capability does not.
    #[test]
    fn a_registered_reply_is_addressable() {
        let mut registry = Registry::default();
        assert!(!registry.is_addressable(REPLY.as_bytes()));

        assert_eq!(registry.register_replies([REPLY.to_string()]), 1);
        assert_eq!(
            registry.register_replies([REPLY.to_string(), EVENT.to_string()]),
            1
        );

        assert!(registry.is_addressable(REPLY.as_bytes()));
        assert!(registry.is_addressable(EVENT.as_bytes()));
        assert_eq!(registry.replies().len(), 2);
    }

    /// Every member spells and numbers itself as the schema does, and reads
    /// back from either; an unknown name or number is nothing.
    #[test]
    fn a_member_reads_from_its_name_or_its_number() {
        fn check<V: Member>() {
            for member in V::ALL {
                assert_eq!(V::from_name(member.name().as_bytes()), Some(*member));
                assert_eq!(V::from_number(member.number()), Some(*member));
            }
            assert_eq!(V::from_name(b"unspecified"), None);
            assert_eq!(V::from_name(b"DURABLE"), None, "names are lowercase");
            assert_eq!(V::from_number(0), None);
        }
        check::<RecordClass>();
        check::<Target>();
        check::<Capability>();
    }
}
