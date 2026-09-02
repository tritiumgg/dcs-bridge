//! The state one DCS process shares, and what a module open does to it.
//!
//! Both DCS Lua states load the broker, so `luaopen_dcsbridge` runs more than
//! once and each state gets its own table. Behind those tables is one
//! [`Bridge`]. It has to be one: the hook driver and the sim driver register
//! from different states, and a per-state map would leave each registrar blind
//! to what the other had done.
//!
//! `Bridge` is where everything process-global lives. The three maps are here,
//! and the rings, the listener and the threads join them as they are built. An
//! open creates none of it, because the first `shim.configure` is what
//! allocates. ADR 0007.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{OnceLock, PoisonError, RwLock, RwLockReadGuard};

/// A topic: the fully-qualified protobuf type name of a record's payload.
///
/// Package names partition the topic space, so the name is the identity and
/// the broker needs no registry to tell two adopters apart.
pub type Topic = String;

/// The drop policy the broker applies to a record under pressure.
///
/// Mirrors `dcs.bridge.RecordClass` in `proto/dcs/bridge/bridge.proto`, whose
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
/// Mirrors `dcs.bridge.Target`. The schema's `UNSPECIFIED` member is resolved
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
/// Mirrors `dcs.bridge.Capability`. That enum is extensible and partitions its
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

/// The three maps the two registrars share.
///
/// They cover different topic sets on purpose. `routes` carries inbound topics
/// only, because routing is what an inbound record needs, while `classes` and
/// `caps` carry every topic that crosses in either direction. So an
/// outbound-only topic has a class and a capability and no route, and that is
/// complete rather than missing something.
///
/// Empty until a registrar fills it, and there is no way to fill it yet: the
/// merge arrives with `shim.classes`, `shim.routes` and `shim.caps`.
#[derive(Debug, Default)]
pub struct Registry {
    classes: HashMap<Topic, RecordClass>,
    routes: HashMap<Topic, Target>,
    caps: HashMap<Topic, Capability>,
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

/// What the DCS process shares between its two Lua states.
///
/// Reached through [`bridge`], which hands out the one instance. Each
/// `luaopen_dcsbridge` gets its own Lua table over this, not its own copy of
/// it.
#[derive(Debug)]
pub struct Bridge {
    opens: AtomicU32,
    registry: RwLock<Registry>,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// The bridge this process shares, creating it on the first call.
///
/// Safe to call from either DCS Lua state and from any thread. Racing callers
/// agree on one instance, and the loser of the race drops its own.
pub fn bridge() -> &'static Bridge {
    BRIDGE.get_or_init(|| Bridge {
        opens: AtomicU32::new(0),
        registry: RwLock::new(Registry::default()),
    })
}

impl Bridge {
    /// Record that a Lua state has opened the module, and number this open.
    ///
    /// The first open is 1. Nothing is allocated and nothing is re-initialized,
    /// so a state that opens the module after the broker is already configured
    /// and running disturbs neither. ADR 0007.
    pub fn open(&self) -> u32 {
        self.opens.fetch_add(1, Ordering::Relaxed).saturating_add(1)
    }

    /// How many Lua states have opened the module in this process.
    pub fn opens(&self) -> u32 {
        self.opens.load(Ordering::Relaxed)
    }

    /// Read the three registration maps.
    ///
    /// A panic while the maps were being written poisons the lock, and this
    /// reads through the poison rather than propagating it. A parser fault is
    /// meant to drop one connection and leave the process running, which a lock
    /// that fails every later reader on someone else's unwind would undo.
    pub fn registry(&self) -> RwLockReadGuard<'_, Registry> {
        self.registry.read().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the process-global rule: two registrars in two Lua
    /// states have to see each other's entries, which they only can through one
    /// instance.
    #[test]
    fn one_bridge_per_process() {
        assert!(
            std::ptr::eq(bridge(), bridge()),
            "two calls to bridge() gave two instances"
        );
    }

    /// The two states do not open the module in lock-step — the hook state opens
    /// at DCS start and the sim state at a mission load — but a Route B sim
    /// driver can open one while another thread is already inside the broker.
    #[test]
    fn racing_first_use_yields_one_bridge() {
        // An address rather than a reference, because a raw pointer does not
        // cross a thread boundary and the identity is all this compares.
        let racers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| std::ptr::from_ref(bridge()) as usize))
            .collect();

        let found: Vec<_> = racers
            .into_iter()
            .map(|racer| racer.join().expect("the thread only calls bridge()"))
            .collect();

        assert!(
            found.windows(2).all(|pair| pair[0] == pair[1]),
            "eight racing callers saw more than one bridge: {found:?}"
        );
    }

    /// Each open is numbered, so a Lua table can say which open produced it and
    /// two tables can be told apart by reading one counter through both.
    ///
    /// The whole test binary is one process and its tests run in parallel, so
    /// this asserts the counter rises rather than pinning it to 1 and 2. The
    /// exact numbers are checked from Lua in `tests/lua/load.lua`, which gets a
    /// process to itself.
    #[test]
    fn each_open_is_numbered() {
        let first = bridge().open();
        let second = bridge().open();

        assert!(
            second > first,
            "a second open numbered {second} against a first of {first}"
        );
        assert!(bridge().opens() >= second, "the count went backwards");
    }

    /// The first `shim.configure` is what allocates a ring or opens a listener.
    /// Opening the module is not that call, and registering is a separate one
    /// after it.
    #[test]
    fn opening_registers_nothing() {
        bridge().open();
        let registry = bridge().registry();

        assert!(registry.classes().is_empty(), "an open registered a class");
        assert!(registry.routes().is_empty(), "an open registered a route");
        assert!(
            registry.caps().is_empty(),
            "an open registered a capability"
        );
    }

    /// The three enums are hand-copied from the schema and their numbers cross
    /// the wire, so a renumber on either side has to be a test failure rather
    /// than a consumer's problem.
    #[test]
    fn the_enums_match_the_schema() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../proto/dcs/bridge/bridge.proto"
        );
        let schema =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));

        let pairs: [(&str, i32); 9] = [
            ("RECORD_CLASS_DURABLE", RecordClass::Durable as i32),
            ("RECORD_CLASS_LOSSY", RecordClass::Lossy as i32),
            ("RECORD_CLASS_COMMAND", RecordClass::Command as i32),
            ("RECORD_CLASS_LIFECYCLE", RecordClass::Lifecycle as i32),
            ("TARGET_SIM_DRIVER", Target::SimDriver as i32),
            ("TARGET_HOOK_DRIVER", Target::HookDriver as i32),
            ("CAPABILITY_READ", Capability::Read as i32),
            ("CAPABILITY_COMMAND", Capability::Command as i32),
            ("CAPABILITY_RELOAD", Capability::Reload as i32),
        ];

        for (member, ours) in pairs {
            let theirs = schema_number(&schema, member)
                .unwrap_or_else(|| panic!("{member} is not in {path}"));
            assert_eq!(theirs, ours, "{member} is {theirs} in the schema");
        }
    }

    /// The number `schema` gives `member`, out of a line reading `NAME = N;`.
    fn schema_number(schema: &str, member: &str) -> Option<i32> {
        schema.lines().map(str::trim).find_map(|line| {
            line.strip_prefix(member)?
                .trim_start()
                .strip_prefix('=')?
                .trim()
                .strip_suffix(';')?
                .parse()
                .ok()
        })
    }
}
