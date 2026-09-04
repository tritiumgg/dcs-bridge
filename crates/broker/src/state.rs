//! The state one DCS process shares, and what a module open does to it.
//!
//! Both DCS Lua states load the broker, so `luaopen_dcsbridge` runs more than
//! once and each state gets its own table. Behind those tables is one
//! [`Bridge`]. It has to be one: the hook driver and the sim driver register
//! from different states, and a per-state map would leave each registrar blind
//! to what the other had done.
//!
//! `Bridge` is where everything process-global lives. The three maps are here,
//! and the outbound path, the writer thread over the commit ring and the
//! listener that fans it out, joins them once [`Bridge::start_outbound`] is
//! called. The inbound rings and the reader thread arrive as they are built.
//! ADR 0007.
//!
//! Both DCS states commit records, and they run on one thread, so the commit
//! ring's one producer is shared between them behind a lock that is never
//! waited on: `try_lock`, with contention refused and counted, because a
//! second thread committing is a defect rather than a case. ADR 0014.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, RwLockReadGuard, TryLockError,
};
use std::time::{Duration, Instant};

use crate::fanout::{Commit, ConnectionId, Writer};
use crate::handshake;
use crate::inbound::{Answers, AuthError, Liveness, Session};
use crate::transport::{Listener, Record};

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

/// The bridge's own acknowledgement record, and the one topic a record may
/// be addressed to before any registration.
///
/// The broker holds no schema, so which topics are replies reaches it by
/// registration, and the registration does not exist yet. The
/// acknowledgement is different: it is the bridge's own message, in the
/// bridge's own package, so the broker knows it by name. ADR 0017.
pub const ACK_TOPIC: &str = "dcs.bridge.CommandAck";

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
        // A topic is a type name, so a topic that is not UTF-8 is registered
        // nowhere and the lookup can say so without a copy.
        topic == ACK_TOPIC.as_bytes()
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

/// What the DCS process shares between its two Lua states.
///
/// Reached through [`bridge`], which hands out the one instance. Each
/// `luaopen_dcsbridge` gets its own Lua table over this, not its own copy of
/// it.
#[derive(Debug)]
pub struct Bridge {
    opens: AtomicU32,
    registry: RwLock<Registry>,
    outbound: OnceLock<Outbound>,
    /// Held while the outbound path is being started, so two starters
    /// cannot both bind.
    starting: Mutex<()>,
    /// Records refused at `begin_to` because their topic is neither a reply
    /// nor the acknowledgement.
    misaddressed: AtomicU64,
    /// Names this process in every handshake, so a consumer can tell a
    /// restarted broker from the one it was talking to.
    instance_id: u64,
    /// When the process started, the origin every heartbeat is measured
    /// from.
    started: Instant,
    /// Milliseconds after `started` at which the logic thread last stamped
    /// the heartbeat, plus one, so that zero means never. Nothing stamps it
    /// yet: `shim.tick` is what will.
    heartbeat: AtomicU64,
    /// The effective value of the `enabled` key, the kill switch.
    enabled: AtomicBool,
    /// The `tokens` key: every consumer credential, replaced whole.
    tokens: RwLock<Vec<Token>>,
    /// Connections authenticated right now, held under `MAX_CONNECTIONS`.
    authenticated: AtomicU64,
}

/// How old the heartbeat may be for the sim to count as alive.
///
/// The specification's default: the largest measured gap while running is
/// under a third of a second and the largest untelegraphed transition under
/// two, so this carries a wide margin over both. The first `configure` is
/// where the configured value arrives.
pub const DCS_ALIVE_THRESHOLD: Duration = Duration::from_millis(30_000);

/// The outbound path: the writer thread, the logic thread's end of the
/// commit ring, and the listener whose connections the writer fans out to.
pub struct Outbound {
    /// Dropping it stops the writer thread, and its count is read here.
    writer: Writer<Record>,
    commit: Mutex<Commit<Record>>,
    listener: Listener,
    contended: AtomicU64,
}

impl fmt::Debug for Outbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outbound")
            .field("listening", &self.listener.local_addr())
            .field("contended", &self.contended.load(Ordering::Relaxed))
            .field("unaddressed", &self.writer.unaddressed())
            .finish_non_exhaustive()
    }
}

impl Outbound {
    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr()
    }

    /// How many commits were refused because another thread held the
    /// commit ring's producer at that moment.
    pub fn contended(&self) -> u64 {
        self.contended.load(Ordering::Relaxed)
    }

    /// How many addressed records were dropped because their connection was
    /// gone by the time the writer thread reached them.
    pub fn unaddressed(&self) -> u64 {
        self.writer.unaddressed()
    }

    /// The commit ring's producer, or why not.
    ///
    /// Never waited on: a second thread committing is a defect rather than a
    /// case, so contention is refused and counted. A panic under the lock
    /// left the producer as it was; the ring itself is sound, so committing
    /// through the poison is right.
    fn producer(&self) -> Result<MutexGuard<'_, Commit<Record>>, CommitError> {
        match self.commit.try_lock() {
            Ok(commit) => Ok(commit),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {
                self.contended.fetch_add(1, Ordering::Relaxed);
                Err(CommitError::Busy)
            }
        }
    }
}

/// Why a record was not queued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitError {
    /// The outbound path has not been started.
    NotStarted,
    /// Another thread was committing. Refused, because waiting would put a
    /// lock on the logic thread and a second committer is a defect.
    Busy,
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CommitError::NotStarted => "the outbound path is not started",
            CommitError::Busy => "another thread is committing",
        })
    }
}

impl std::error::Error for CommitError {}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// The bridge this process shares, creating it on the first call.
///
/// Safe to call from either DCS Lua state and from any thread. Racing callers
/// agree on one instance, and the loser of the race drops its own.
pub fn bridge() -> &'static Bridge {
    BRIDGE.get_or_init(|| Bridge {
        opens: AtomicU32::new(0),
        registry: RwLock::new(Registry::default()),
        outbound: OnceLock::new(),
        starting: Mutex::new(()),
        misaddressed: AtomicU64::new(0),
        instance_id: handshake::instance_id(),
        started: Instant::now(),
        heartbeat: AtomicU64::new(0),
        enabled: AtomicBool::new(true),
        tokens: RwLock::new(Vec::new()),
        authenticated: AtomicU64::new(0),
    })
}

/// The process's bridge, as the reader thread asks it things.
///
/// The bridge lives in a static and the listener wants something it can
/// share between threads, so this stands in for it and forwards every call
/// to [`bridge`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Global;

impl Answers for Global {
    fn handshake(&self) -> Record {
        bridge().handshake().encode()
    }

    fn liveness(&self) -> Liveness {
        bridge().liveness()
    }

    fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
        bridge().authenticate(secret)
    }

    fn disconnected(&self, session: &Session) {
        bridge().disconnected(session);
    }
}

/// One entry of the `tokens` key: a consumer's credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    /// Names the token in stats and audit lines. Never the secret.
    pub id: String,
    /// What a consumer presents in `Auth`.
    pub secret: Vec<u8>,
    /// What the token grants. A token granting nothing is refused at
    /// `Auth`, because it is a configuration mistake and not a consumer's.
    pub caps: HashSet<Capability>,
}

/// How many connections may be authenticated at once. The specification's
/// default: a bot, a map, a stats collector, and headroom. The first
/// `configure` is where the configured value arrives.
pub const MAX_CONNECTIONS: usize = 8;

/// Whether two secrets are the same, in time that depends on their lengths
/// and on nothing else about them.
///
/// Every byte is compared whether or not an earlier one differed, so a
/// wrong secret takes as long as a right one and a guess learns nothing
/// from the clock. Two secrets of different lengths are different, and the
/// comparison still runs over the presented one.
fn same_secret(presented: &[u8], configured: &[u8]) -> bool {
    let mut differ = u8::from(presented.len() != configured.len());
    let against = if presented.len() == configured.len() {
        configured
    } else {
        presented
    };
    for (a, b) in presented.iter().zip(against) {
        differ |= a ^ b;
    }
    differ == 0
}

impl Bridge {
    /// Start the outbound path, or return the address it is already bound
    /// to: the writer thread over a commit ring of `commit_capacity` records,
    /// and a listener on `addr` giving each connection a ring of
    /// `ring_capacity` records.
    ///
    /// The bind is what fails, and it fails with nothing started. A second
    /// call, from the other Lua state or a racing thread, changes nothing
    /// and returns the first call's address.
    pub fn start_outbound(
        &self,
        addr: impl ToSocketAddrs,
        commit_capacity: usize,
        ring_capacity: usize,
    ) -> io::Result<SocketAddr> {
        let _starting = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(outbound) = self.outbound.get() {
            return Ok(outbound.local_addr());
        }

        let (writer, commit, connections) = Writer::spawn(commit_capacity);
        // Answered through the global, so a handshake field that arrives
        // after the listener is up is in the next connection's.
        let listener = Listener::spawn(addr, connections, ring_capacity, Arc::new(Global))?;
        let addr = listener.local_addr();
        // The lock above makes this the only setter.
        let _ = self.outbound.set(Outbound {
            writer,
            commit: Mutex::new(commit),
            listener,
            contended: AtomicU64::new(0),
        });

        Ok(addr)
    }

    /// The outbound path, once started.
    pub fn outbound(&self) -> Option<&Outbound> {
        self.outbound.get()
    }

    /// What this broker greets a connection with, as of now.
    ///
    /// The schema hash is absent until the hook driver hands the schema
    /// over, and there is no way to hand it over yet.
    pub fn handshake(&self) -> handshake::Handshake {
        handshake::Handshake {
            protocol: crate::PROTOCOL_VERSION,
            broker: crate::BROKER_VERSION,
            instance_id: self.instance_id,
            schema_sha256: None,
        }
    }

    /// Stamp the heartbeat: the logic thread is running now.
    ///
    /// One atomic store, and no throttle here: the caller that exists to
    /// throttle it, `shim.tick`, does not exist yet, so nothing calls this
    /// outside a test and every `Pong` reports the sim as never heard from.
    pub fn heartbeat(&self) {
        let ms = self.started.elapsed().as_millis();
        // Saturating at the top of a u64 is 584 million years of uptime.
        let ms = u64::try_from(ms).unwrap_or(u64::MAX - 1);
        self.heartbeat.store(ms + 1, Ordering::Relaxed);
    }

    /// What a `Pong` carries now: the heartbeat's age, whether that is
    /// under the threshold, and the kill switch's effective value. Read on
    /// the reader thread, and it touches nothing the logic thread holds.
    pub fn liveness(&self) -> Liveness {
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX - 1);
        let last_heard_ms = match self.heartbeat.load(Ordering::Relaxed) {
            0 => None,
            stamped => Some(now.saturating_sub(stamped - 1)),
        };
        let threshold = u64::try_from(DCS_ALIVE_THRESHOLD.as_millis()).unwrap_or(u64::MAX);
        Liveness {
            last_heard_ms,
            alive: last_heard_ms.is_some_and(|age| age < threshold),
            enabled: self.enabled.load(Ordering::Relaxed),
        }
    }

    /// The effective value of the `enabled` key.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Replace the token table whole.
    ///
    /// Live: a connection authenticates against whatever the table holds
    /// at that moment. A session authenticated under a token the new table
    /// drops is not closed here; revocation is a later task's.
    pub fn set_tokens(&self, tokens: Vec<Token>) {
        *self.tokens.write().unwrap_or_else(PoisonError::into_inner) = tokens;
    }

    /// Match `secret` against every token and open a session on the one
    /// that carries it.
    ///
    /// Every token is compared whether or not an earlier one matched, so
    /// the time taken says nothing about which entry, if any, was right.
    /// A match with an empty capability set is refused: the token can do
    /// nothing, which is a configuration mistake and worth a distinct error.
    /// A match past `MAX_CONNECTIONS` authenticated sessions is refused as
    /// full, and the count is taken here so two racing `Auth`s cannot both
    /// take the last slot.
    pub fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
        let tokens = self.tokens.read().unwrap_or_else(PoisonError::into_inner);
        let mut matched = None;
        for token in tokens.iter() {
            if same_secret(secret, &token.secret) && matched.is_none() {
                matched = Some(token);
            }
        }
        let token = matched.ok_or(AuthError::BadToken)?;
        if token.caps.is_empty() {
            return Err(AuthError::EmptyCapabilitySet);
        }

        let limit = MAX_CONNECTIONS as u64;
        let mut held = self.authenticated.load(Ordering::Relaxed);
        loop {
            if held >= limit {
                return Err(AuthError::ServerFull);
            }
            match self.authenticated.compare_exchange_weak(
                held,
                held + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(now) => held = now,
            }
        }

        Ok(Session {
            token_id: token.id.clone(),
            caps: token.caps.clone(),
        })
    }

    /// A session's connection has closed; its slot is free.
    pub fn disconnected(&self, _session: &Session) {
        self.authenticated.fetch_sub(1, Ordering::Relaxed);
    }

    /// How many connections are authenticated right now.
    pub fn authenticated(&self) -> u64 {
        self.authenticated.load(Ordering::Relaxed)
    }

    /// Queue an envelope tail for every connection.
    ///
    /// The tail is copied once, into the allocation the rings share by
    /// reference; that is the one allocation on the commit path. A record the
    /// commit ring evicts to make room comes back here and is dropped on the
    /// calling thread. ADR 0014.
    pub fn commit(&self, tail: &[u8]) -> Result<(), CommitError> {
        let outbound = self.outbound.get().ok_or(CommitError::NotStarted)?;
        let record: Record = Arc::from(tail);

        let mut commit = outbound.producer()?;
        drop(commit.push(record));
        Ok(())
    }

    /// Queue an envelope tail for one connection and no other.
    ///
    /// Queued the same way [`commit`](Self::commit) queues, and with the same
    /// one allocation. Whether `to` is still attached is not checked here:
    /// the writer thread is the one that knows, and it drops and counts a
    /// record whose connection has gone, which [`Outbound::unaddressed`]
    /// reports. So a record addressed to a closed connection returns `Ok`.
    pub fn commit_to(&self, to: ConnectionId, tail: &[u8]) -> Result<(), CommitError> {
        let outbound = self.outbound.get().ok_or(CommitError::NotStarted)?;
        let record: Record = Arc::from(tail);

        let mut commit = outbound.producer()?;
        drop(commit.push_to(to, record));
        Ok(())
    }

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

    /// Whether a record on `topic` may be addressed to one connection,
    /// counting a refusal.
    ///
    /// The answer is a plain boolean with no lock held by the time it
    /// returns, on purpose: the Lua side raises an error on `false`, and Lua
    /// raises with `longjmp`, which runs no Rust drop. A read guard alive
    /// across that jump would never release, and the first registrar to want
    /// the write lock would wait forever on the logic thread. Taking the read
    /// lock blocks only against a writer, and the only writers are the
    /// registrars on this same thread, so it never waits.
    pub fn addressable(&self, topic: &[u8]) -> bool {
        let addressable = self.registry().is_addressable(topic);
        if !addressable {
            self.misaddressed.fetch_add(1, Ordering::Relaxed);
        }
        addressable
    }

    /// How many `begin_to` calls were refused for naming a topic that is
    /// neither a reply nor the acknowledgement. Always hand-written Lua: the
    /// generator addresses only what the schema marks.
    pub fn misaddressed(&self) -> u64 {
        self.misaddressed.load(Ordering::Relaxed)
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

        let pairs: [(&str, i32); 12] = [
            ("AUTH_ERROR_BAD_TOKEN", AuthError::BadToken as i32),
            (
                "AUTH_ERROR_EMPTY_CAPABILITY_SET",
                AuthError::EmptyCapabilitySet as i32,
            ),
            ("AUTH_ERROR_SERVER_FULL", AuthError::ServerFull as i32),
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

        // The acknowledgement and the handshake are known by name, so each
        // name has to be the schema's: the package the file declares and
        // the message it holds.
        let handshake = std::str::from_utf8(handshake::TOPIC).expect("the topic is a name");
        for topic in [ACK_TOPIC, handshake] {
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

    /// A secret no token carries is a bad token, a token granting nothing
    /// is refused as such, the count of sessions is held under the cap and
    /// comes back down at disconnect, and a secret of the wrong length is
    /// wrong.
    #[test]
    fn authentication_matches_a_token_and_holds_the_session_count() {
        let bridge = Bridge {
            opens: AtomicU32::new(0),
            registry: RwLock::new(Registry::default()),
            outbound: OnceLock::new(),
            starting: Mutex::new(()),
            misaddressed: AtomicU64::new(0),
            instance_id: 1,
            started: Instant::now(),
            heartbeat: AtomicU64::new(0),
            enabled: AtomicBool::new(true),
            tokens: RwLock::new(Vec::new()),
            authenticated: AtomicU64::new(0),
        };
        assert_eq!(bridge.authenticate(b"anything"), Err(AuthError::BadToken));

        bridge.set_tokens(vec![
            Token {
                id: "reader".into(),
                secret: b"correct horse".to_vec(),
                caps: [Capability::Read].into_iter().collect(),
            },
            Token {
                id: "useless".into(),
                secret: b"battery staple".to_vec(),
                caps: HashSet::new(),
            },
        ]);
        assert_eq!(
            bridge.authenticate(b"correct hors"),
            Err(AuthError::BadToken)
        );
        assert_eq!(
            bridge.authenticate(b"correct horses"),
            Err(AuthError::BadToken)
        );
        assert_eq!(
            bridge.authenticate(b"battery staple"),
            Err(AuthError::EmptyCapabilitySet)
        );
        assert_eq!(bridge.authenticated(), 0, "a refusal took a slot");

        let sessions: Vec<Session> = (0..MAX_CONNECTIONS)
            .map(|_| {
                bridge
                    .authenticate(b"correct horse")
                    .expect("under the cap")
            })
            .collect();
        assert_eq!(sessions[0].token_id, "reader");
        assert_eq!(
            sessions[0].caps,
            [Capability::Read].into_iter().collect::<HashSet<_>>()
        );
        assert_eq!(
            bridge.authenticate(b"correct horse"),
            Err(AuthError::ServerFull)
        );

        bridge.disconnected(&sessions[0]);
        assert!(
            bridge.authenticate(b"correct horse").is_ok(),
            "a freed slot was not reused"
        );

        assert!(same_secret(b"", b""));
        assert!(!same_secret(b"a", b""));
        assert!(!same_secret(b"", b"a"));
    }

    /// Before any registration the acknowledgement is the one addressable
    /// topic, a fan-out topic is refused, and only the refusal is counted.
    #[test]
    fn the_acknowledgement_is_addressable_and_a_fan_out_topic_is_refused() {
        let before = bridge().misaddressed();

        assert!(bridge().addressable(ACK_TOPIC.as_bytes()));
        assert_eq!(
            bridge().misaddressed(),
            before,
            "an accepted address was counted"
        );

        assert!(!bridge().addressable(b"dcs.builtin.UnitDestroyed"));
        assert!(!bridge().addressable(b""));
        assert!(
            bridge().misaddressed() >= before + 2,
            "two refusals were not counted as two"
        );
    }

    /// The outbound path starts once, and a record committed after that
    /// reaches a connection as a frame carrying that connection's `seq`.
    ///
    /// The bridge is the process's, so this shares it with every other test
    /// in the binary; none of the others starts the outbound path or commits,
    /// and a second start returns the first's address rather than failing.
    #[test]
    fn the_outbound_path_starts_once_and_commits_reach_a_connection() {
        use std::io::Read;
        use std::net::TcpStream;
        use std::time::Duration;

        let addr = bridge()
            .start_outbound("127.0.0.1:0", 64, 64)
            .expect("loopback binds");
        assert_eq!(
            bridge().start_outbound("127.0.0.1:0", 64, 64).unwrap(),
            addr,
            "a second start bound a second listener"
        );
        assert_eq!(bridge().outbound().unwrap().local_addr(), addr);

        let mut client = TcpStream::connect(addr).expect("the listener accepts");
        client
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();

        // The handshake comes first, numbered 1, carrying this bridge's
        // instance id and no schema hash.
        let mut length = [0u8; 4];
        client
            .read_exact(&mut length)
            .expect("the handshake arrives");
        let mut frame = vec![0u8; u32::from_le_bytes(length) as usize];
        client
            .read_exact(&mut frame)
            .expect("the handshake's body arrives");
        let greeting = bridge().handshake().encode();
        assert_eq!(&frame[..2], [0x08, 0x01], "the handshake is not seq 1");
        assert_eq!(
            &frame[2..],
            &greeting[..],
            "the handshake is not this bridge's"
        );
        assert_eq!(
            bridge().handshake(),
            handshake::Handshake {
                protocol: crate::PROTOCOL_VERSION,
                broker: crate::BROKER_VERSION,
                instance_id: bridge().handshake().instance_id,
                schema_sha256: None,
            }
        );

        // The handshake is queued after the attach, so a record committed
        // now reaches the connection, numbered after it.
        let tail = [0x22, 0x00];
        bridge().commit(&tail).expect("the path is started");
        client.read_exact(&mut length).expect("a frame arrives");
        assert_eq!(u32::from_le_bytes(length), 2 + tail.len() as u32);
        let mut frame = vec![0u8; u32::from_le_bytes(length) as usize];
        client
            .read_exact(&mut frame)
            .expect("the frame's body arrives");
        assert_eq!(frame, [0x08, 0x02, 0x22, 0x00], "seq 2 then the tail");
        assert_eq!(bridge().outbound().unwrap().contended(), 0);

        // A record addressed to a connection that does not exist is queued,
        // and the writer thread is where it is dropped and counted. This is
        // the one addressed commit in the binary against the shared bridge.
        bridge()
            .commit_to(ConnectionId::from_raw(u64::MAX), &tail)
            .expect("an address is not checked at commit");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while bridge().outbound().unwrap().unaddressed() < 1 {
            assert!(
                std::time::Instant::now() < deadline,
                "the record with nowhere to go was not counted"
            );
            std::thread::yield_now();
        }
        assert_eq!(bridge().outbound().unwrap().unaddressed(), 1);
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
