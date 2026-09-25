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

use crate::fanout::{Class, TopicName};

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

impl RecordClass {
    /// The ring a record of this class takes outbound.
    ///
    /// `Command` is an inbound class, and a topic registered under it may
    /// still be committed outbound. Such a record is not to be lost
    /// silently and marks no boundary, so it goes with `Durable`. This is
    /// the one place that says so. ADR 0028.
    pub const fn outbound(self) -> Class {
        match self {
            Self::Lossy => Class::Lossy,
            Self::Durable | Self::Command => Class::Durable,
            Self::Lifecycle => Class::Lifecycle,
        }
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
pub enum Refusal {
    /// A row named a topic with a value other than the one held, by the
    /// map or by an earlier row of the same call.
    Conflict {
        /// The topic the call named.
        topic: Topic,
        /// What the topic is registered as.
        held: &'static str,
        /// What the call offered instead.
        offered: &'static str,
    },
    /// The call's fresh `LIFECYCLE` topics would take the retained set
    /// past `max_lifecycle_topics`. Every slot is allocated at the first
    /// configure, so there is no room to make. ADR 0029.
    OverCap {
        /// `LIFECYCLE` topics already bound to a slot.
        bound: u32,
        /// `LIFECYCLE` topics the call would bind.
        offered: u32,
        /// `max_lifecycle_topics`.
        cap: u32,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Conflict {
                topic,
                held,
                offered,
            } => write!(f, "{topic} is registered as {held}, not {offered}"),
            Refusal::OverCap {
                bound,
                offered,
                cap,
            } => write!(
                f,
                "{offered} LIFECYCLE topics beside {bound} bound would exceed max_lifecycle_topics {cap}"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

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
) -> Result<usize, Refusal> {
    let fresh = fresh(map, rows)?;
    let added = fresh.len();
    map.extend(fresh);
    Ok(added)
}

/// [`merge`]'s first pass: the rows new to `map`, or the conflict that
/// refuses the call.
fn fresh<V: Member>(
    map: &HashMap<Topic, V>,
    rows: impl IntoIterator<Item = (Topic, V)>,
) -> Result<HashMap<Topic, V>, Refusal> {
    let mut fresh: HashMap<Topic, V> = HashMap::new();
    for (topic, offered) in rows {
        match map.get(&topic).or_else(|| fresh.get(&topic)) {
            Some(held) if *held != offered => {
                return Err(Refusal::Conflict {
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
    Ok(fresh)
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
///
/// A `LIFECYCLE` topic is bound to a slot in the retained set as its class
/// arrives, and the slot is what the record carries to the writer thread,
/// which holds no registry. Slots are never given back: retiring a topic
/// is a DCS restart. ADR 0029.
///
/// The topic's name travels the same way, for the writer thread's topic
/// filters to match: one shared allocation per topic, made as its class
/// arrives, so a record carries a reference count and not a copy. ADR 0030.
#[derive(Debug, Default)]
pub struct Registry {
    classes: HashMap<Topic, RecordClass>,
    routes: HashMap<Topic, Target>,
    caps: HashMap<Topic, Capability>,
    replies: HashSet<Topic>,
    /// The retained-set slot of every `LIFECYCLE` topic in `classes`.
    slots: HashMap<Topic, u32>,
    /// The shared name of every topic in `classes`.
    names: HashMap<Topic, TopicName>,
}

impl Registry {
    /// Merge a table of drop policies, binding each fresh `LIFECYCLE` topic
    /// to the next slot under `cap`. See [`merge`].
    ///
    /// A call whose fresh `LIFECYCLE` topics would take the bound count
    /// past `cap` is refused whole, the row of another class beside them
    /// included: the two-pass shape of `merge`, so a registrar's table is
    /// applied or it is not. A topic already bound binds nothing again.
    pub fn register_classes(
        &mut self,
        rows: impl IntoIterator<Item = (Topic, RecordClass)>,
        cap: u32,
    ) -> Result<usize, Refusal> {
        let fresh = fresh(&self.classes, rows)?;
        let bound = self.slots.len() as u32;
        let offered = fresh
            .values()
            .filter(|class| **class == RecordClass::Lifecycle)
            .count() as u32;
        if bound.saturating_add(offered) > cap {
            return Err(Refusal::OverCap {
                bound,
                offered,
                cap,
            });
        }
        let added = fresh.len();
        for (topic, class) in fresh {
            if class == RecordClass::Lifecycle {
                // Under the cap, which is a u32.
                let slot = self.slots.len() as u32;
                self.slots.insert(topic.clone(), slot);
            }
            self.names
                .insert(topic.clone(), TopicName::from(topic.as_str()));
            self.classes.insert(topic, class);
        }
        Ok(added)
    }

    /// Merge a table of destination states. See [`merge`].
    pub fn register_routes(
        &mut self,
        rows: impl IntoIterator<Item = (Topic, Target)>,
    ) -> Result<usize, Refusal> {
        merge(&mut self.routes, rows)
    }

    /// Merge a table of required capabilities. See [`merge`].
    pub fn register_caps(
        &mut self,
        rows: impl IntoIterator<Item = (Topic, Capability)>,
    ) -> Result<usize, Refusal> {
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
        self.required(topic).is_some()
    }

    /// The capability a connection needs to receive a record on `topic`,
    /// the ring the record takes on its way there, and the retained-set
    /// slot a `LIFECYCLE` record replaces, or `None` when the topic is
    /// incomplete by [`is_complete`](Self::is_complete)'s rule.
    ///
    /// Looked up once, when the record is opened, and carried with it to
    /// fan-out: the writer thread holds no registry. The acknowledgement
    /// answers `command`, so that is what covers it; it is addressed to the
    /// one connection that sent the command, and an addressed record is
    /// never filtered, so the value is not consulted on that path. It is
    /// `DURABLE` in the schema. ADR 0028.
    pub fn required(&self, topic: &[u8]) -> Option<(Capability, Class, Option<u32>, TopicName)> {
        if topic == dcsbridge_topic::COMMAND_ACK.as_bytes() {
            // An addressed record's name is never matched, so the
            // allocation here is one nothing reads; it is one per
            // acknowledgement, on the path that opened an encoder.
            return Some((
                Capability::Command,
                Class::Durable,
                None,
                TopicName::from(dcsbridge_topic::COMMAND_ACK),
            ));
        }
        let topic = std::str::from_utf8(topic).ok()?;
        let class = self.classes.get(topic)?.outbound();
        let slot = self.slots.get(topic).copied();
        let name = TopicName::clone(self.names.get(topic)?);
        Some((self.caps.get(topic).copied()?, class, slot, name))
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

    /// A cap no test here reaches, unless it is about the cap.
    const CAP: u32 = 64;

    fn rows<V: Member>(rows: &[(&str, V)]) -> Vec<(Topic, V)> {
        rows.iter().map(|(t, v)| (t.to_string(), *v)).collect()
    }

    /// `count` distinct `LIFECYCLE` topics, numbered from `from`.
    fn lifecycle(from: u32, count: u32) -> Vec<(Topic, RecordClass)> {
        (from..from + count)
            .map(|n| {
                (
                    format!("dcsbridge.builtin.hook.Boundary{n}"),
                    RecordClass::Lifecycle,
                )
            })
            .collect()
    }

    /// Two registrars over disjoint topic sets both merge, and the map
    /// holds the union.
    #[test]
    fn a_second_registrar_over_new_topics_merges() {
        let mut registry = Registry::default();

        let hook = registry.register_classes(rows(&[(EVENT, RecordClass::Durable)]), CAP);
        let sim = registry.register_classes(rows(&[(COMMAND, RecordClass::Command)]), CAP);

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
            Err(Refusal::Conflict {
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

        let refused = registry.register_classes(
            rows(&[(EVENT, RecordClass::Durable), (EVENT, RecordClass::Lossy)]),
            CAP,
        );

        assert_eq!(
            refused,
            Err(Refusal::Conflict {
                topic: EVENT.into(),
                held: "durable",
                offered: "lossy",
            })
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
            .register_classes(
                rows(&[
                    (EVENT, RecordClass::Durable),
                    (COMMAND, RecordClass::Command),
                ]),
                CAP,
            )
            .expect("classes");
        registry
            .register_caps(rows(&[(EVENT, Capability::Read)]))
            .expect("caps");
        registry
            .register_routes(rows(&[(COMMAND, Target::SimDriver)]))
            .expect("routes");

        assert!(registry.is_complete(EVENT.as_bytes()));
        assert_eq!(
            registry.required(EVENT.as_bytes()),
            Some((
                Capability::Read,
                Class::Durable,
                None,
                TopicName::from(EVENT)
            ))
        );
        assert!(
            !registry.is_complete(COMMAND.as_bytes()),
            "a class and a route made a topic complete without a capability"
        );
        assert!(!registry.is_complete(b"dcsbridge.builtin.sim.Unregistered"));
        assert!(!registry.is_complete(b"\xff"));

        // A capability with no class is as incomplete as the reverse.
        let mut classless = Registry::default();
        classless
            .register_caps(rows(&[(EVENT, Capability::Read)]))
            .expect("caps");
        assert_eq!(classless.required(EVENT.as_bytes()), None);
    }

    /// The name a complete topic's records carry is one allocation shared
    /// by every lookup, so a commit costs a count and not a copy.
    #[test]
    fn a_topic_name_is_shared_across_lookups() {
        let mut registry = Registry::default();
        registry
            .register_classes(rows(&[(EVENT, RecordClass::Durable)]), CAP)
            .expect("classes");
        registry
            .register_caps(rows(&[(EVENT, Capability::Read)]))
            .expect("caps");

        let (_, _, _, first) = registry.required(EVENT.as_bytes()).expect("complete");
        let (_, _, _, second) = registry.required(EVENT.as_bytes()).expect("complete");
        assert_eq!(&*first, EVENT);
        assert!(
            TopicName::ptr_eq(&first, &second),
            "each lookup allocated the name again"
        );
    }

    /// Each class names its own ring, and a command-class topic committed
    /// outbound takes the durable one. The one `LIFECYCLE` topic is bound
    /// to the first slot, and no other topic has one.
    #[test]
    fn a_topic_takes_the_ring_of_its_class_and_a_command_the_durable_one() {
        const GAUGE: &str = "dcsbridge.builtin.sim.Gauge";
        const EDGE: &str = "dcsbridge.builtin.hook.Edge";
        let all = [
            (GAUGE, RecordClass::Lossy, Class::Lossy, None),
            (EVENT, RecordClass::Durable, Class::Durable, None),
            (EDGE, RecordClass::Lifecycle, Class::Lifecycle, Some(0)),
            (COMMAND, RecordClass::Command, Class::Durable, None),
        ];
        let mut registry = Registry::default();
        for (topic, class, _, _) in all {
            registry
                .register_classes(rows(&[(topic, class)]), CAP)
                .expect("classes");
            registry
                .register_caps(rows(&[(topic, Capability::Read)]))
                .expect("caps");
        }

        for (topic, _, ring, slot) in all {
            assert_eq!(
                registry.required(topic.as_bytes()),
                Some((Capability::Read, ring, slot, TopicName::from(topic))),
                "{topic}"
            );
        }
        assert_eq!(
            registry.required(dcsbridge_topic::COMMAND_ACK.as_bytes()),
            Some((
                Capability::Command,
                Class::Durable,
                None,
                TopicName::from(dcsbridge_topic::COMMAND_ACK)
            ))
        );
    }

    /// The cap bounds how many `LIFECYCLE` topics are ever bound. A call
    /// past it is refused whole, the row of another class beside it
    /// unapplied, and re-registering bound topics binds nothing more. The
    /// slot numbers within one call are not asserted, because `merge`
    /// hands its fresh rows over in hash order; what holds is that the
    /// bound slots are distinct and all under the cap.
    #[test]
    fn a_lifecycle_registration_past_the_cap_is_refused_whole() {
        let mut registry = Registry::default();

        assert_eq!(registry.register_classes(lifecycle(0, CAP), CAP), Ok(64));
        assert_eq!(
            registry.register_classes(lifecycle(0, CAP), CAP),
            Ok(0),
            "a re-registration bound a slot"
        );

        let mut over = lifecycle(CAP, 1);
        over.push((EVENT.to_string(), RecordClass::Durable));
        assert_eq!(
            registry.register_classes(over, CAP),
            Err(Refusal::OverCap {
                bound: 64,
                offered: 1,
                cap: 64,
            })
        );
        assert_eq!(
            registry.classes().len(),
            64,
            "a refused call applied the durable row beside the cap"
        );
        assert_eq!(
            Refusal::OverCap {
                bound: 64,
                offered: 1,
                cap: 64
            }
            .to_string(),
            "1 LIFECYCLE topics beside 64 bound would exceed max_lifecycle_topics 64"
        );

        let mut slots: Vec<u32> = registry.slots.values().copied().collect();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), 64, "two topics share a slot");
        assert!(slots.iter().all(|slot| *slot < CAP), "a slot past the cap");
    }

    /// One call of one more than the cap is refused with nothing bound,
    /// under a cap smaller than the default.
    #[test]
    fn one_call_over_a_small_cap_binds_nothing() {
        let mut registry = Registry::default();

        assert_eq!(
            registry.register_classes(lifecycle(0, 17), 16),
            Err(Refusal::OverCap {
                bound: 0,
                offered: 17,
                cap: 16,
            })
        );
        assert!(
            registry.classes().is_empty(),
            "a refused call applied a row"
        );
        assert_eq!(registry.register_classes(lifecycle(0, 16), 16), Ok(16));
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
