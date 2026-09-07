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
use std::time::Instant;

use crate::config::{self, Applied, Config, Value};
use crate::fanout::{Commit, ConnectionId, Writer};
use crate::handshake;
use crate::inbound::{Answers, AuthError, Limits, Liveness, Session};
use crate::transport::{Listener, Record};

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
    /// the heartbeat, plus one, so that zero means never. `shim.tick`
    /// stamps it, at most once per `heartbeat_interval_ms`.
    heartbeat: AtomicU64,
    /// The mission time `shim.tick` last published, as the bits of an
    /// `f64`. The sim owns this clock and it pauses with the sim; no wall
    /// clock stands in for it.
    mission_time: AtomicU64,
    /// Whether a mission load is in progress, which picks the liveness
    /// threshold.
    loading: AtomicBool,
    /// The configuration in force, replaced whole by each `configure` and
    /// by `SetEnabled`, which moves the `enabled` key inside it. Every
    /// thread that decides by a live key reads it here, taking the lock for
    /// the length of one pointer copy. ADR 0019.
    config: RwLock<Arc<Config>>,
    /// Held by every writer of `config` from its read of the configuration
    /// in force to its swap, so two writers cannot each build on the same
    /// old one and the second lose the first. Holds whether the first
    /// `configure` has happened, read under the same lock. Before it, every
    /// value is the specification's default and nothing is allocated from
    /// one.
    configuring: Mutex<bool>,
    /// `config_keys_pending_restart`: restart-tier keys whose file value
    /// differs from the one in force, as of the last `configure`.
    pending_restart: AtomicU64,
    /// Keys the last `configure` carried that the broker does not own.
    unknown_keys: AtomicU64,
    /// Connections authenticated right now, held under `max_connections`.
    authenticated: AtomicU64,
    /// `SeqAck` records consumed. Nothing reads the number they carry until
    /// the replay spool exists.
    seq_acks: AtomicU64,
    /// Messages refused because the session's token lacked the capability
    /// they require. `commands_rejected_total` by that reason, once stats
    /// exist.
    no_capability: AtomicU64,
    /// The schema the hook driver handed over, held for the life of the
    /// process: replacing the served set is a DCS restart, so a second
    /// hand-off is refused rather than applied.
    schema: OnceLock<HeldSchema>,
}

/// The schema as the broker holds it.
///
/// The broker parses none of the bytes; it holds them, hashes them once,
/// and hands them to the reader thread that answers a `GetSchema`, which
/// wraps them in the answer. Shared by reference, so a request copies
/// nothing out of here.
struct HeldSchema {
    /// The compiled `FileDescriptorSet`, as handed over.
    set: Record,
    /// The SHA-256 of the set, which every handshake carries from now on.
    sha256: [u8; 32],
}

impl fmt::Debug for HeldSchema {
    /// The length and the hash: a panic message that printed the bridge
    /// would otherwise carry the whole set as a byte list.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeldSchema")
            .field("len", &self.set.len())
            .field("sha256", &self.sha256)
            .finish()
    }
}

/// Why a schema hand-off was refused, with nothing held.
#[derive(Debug, Eq, PartialEq)]
pub enum SchemaError {
    /// The first `configure` has not happened.
    NotConfigured,
    /// The bytes were empty, which is a file that was not read.
    Empty,
    /// The `Schema` answer would outgrow the frame cap in force, so no
    /// consumer could be served it.
    TooLarge {
        /// The set's length.
        len: usize,
        /// `max_frame_bytes` as of the call.
        max_frame_bytes: u32,
    },
    /// A schema is held already.
    Held,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::NotConfigured => {
                f.write_str("configure comes first: the schema is handed over after it")
            }
            SchemaError::Empty => f.write_str("the schema is empty"),
            SchemaError::TooLarge {
                len,
                max_frame_bytes,
            } => write!(
                f,
                "the schema is {len} bytes and its answer would not fit max_frame_bytes {max_frame_bytes}"
            ),
            SchemaError::Held => {
                f.write_str("a schema is held already: replacing the served set is a DCS restart")
            }
        }
    }
}

impl std::error::Error for SchemaError {}

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

/// Why a `configure` was refused, with nothing changed.
#[derive(Debug)]
pub enum ConfigureError {
    /// The table failed a check.
    Config(config::Error),
    /// The first call could not bind its listener.
    Bind {
        /// The address the table named.
        addr: SocketAddr,
        /// What the bind said.
        error: io::Error,
    },
}

impl From<config::Error> for ConfigureError {
    fn from(error: config::Error) -> Self {
        ConfigureError::Config(error)
    }
}

impl fmt::Display for ConfigureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigureError::Config(error) => error.fmt(f),
            ConfigureError::Bind { addr, error } => write!(f, "cannot listen on {addr}: {error}"),
        }
    }
}

impl std::error::Error for ConfigureError {}

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
    BRIDGE.get_or_init(|| Bridge::new(handshake::instance_id()))
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

    fn limits(&self) -> Limits {
        Limits::from(&*bridge().config())
    }

    fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
        bridge().authenticate(secret)
    }

    fn disconnected(&self, session: &Session) {
        bridge().disconnected(session);
    }

    fn schema(&self) -> Option<Record> {
        bridge().schema()
    }

    fn seq_ack(&self, seq: u64) {
        bridge().seq_ack(seq);
    }

    fn set_enabled(&self, enabled: bool) {
        bridge().set_enabled(enabled);
    }

    fn refused_no_capability(&self, topic: &str) {
        bridge().refused_no_capability(topic);
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

/// Whether two secrets are the same, in time that depends on the presented
/// secret's length and on nothing about either secret's content.
///
/// Every byte is compared whether or not an earlier one differed, so a
/// wrong secret takes as long as a right one and a guess learns nothing
/// from the clock. Two secrets of different lengths are different, and the
/// comparison still runs over the presented one, against itself. What the
/// clock can tell is whether the lengths matched, since only then is the
/// configured secret's memory touched; a length is not a secret.
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
    /// A bridge with nothing started, nothing registered, and the
    /// specification's defaults in force.
    fn new(instance_id: u64) -> Self {
        Bridge {
            opens: AtomicU32::new(0),
            registry: RwLock::new(Registry::default()),
            outbound: OnceLock::new(),
            starting: Mutex::new(()),
            misaddressed: AtomicU64::new(0),
            instance_id,
            started: Instant::now(),
            heartbeat: AtomicU64::new(0),
            mission_time: AtomicU64::new(0),
            loading: AtomicBool::new(false),
            config: RwLock::new(Arc::new(Config::default())),
            configuring: Mutex::new(false),
            pending_restart: AtomicU64::new(0),
            unknown_keys: AtomicU64::new(0),
            authenticated: AtomicU64::new(0),
            seq_acks: AtomicU64::new(0),
            no_capability: AtomicU64::new(0),
            schema: OnceLock::new(),
        }
    }

    /// The configuration in force, as of now.
    ///
    /// A pointer copy under the read lock, so a thread deciding by a live
    /// key holds the lock for no longer than that and sees one whole
    /// configuration, never half of one and half of the next.
    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Apply a configuration table as one swap, or refuse it whole and
    /// change nothing.
    ///
    /// The first call applies every key. A later one applies the live keys,
    /// counts a changed restart-tier key as pending a restart, and leaves
    /// it as it is. Both count the keys the broker does not own. The kill
    /// switch takes the table's `enabled`, and the token table is replaced,
    /// so a connection authenticating after this sees the new one; a
    /// session opened under a token the table drops is not closed here.
    ///
    /// The first call is also what allocates and binds: the commit ring,
    /// the listener on `bind_address` and `port`, and the ring each
    /// connection gets, all sized from the table. A bind that fails refuses
    /// the call with nothing in force and nothing started, so the hook
    /// driver can fix the address and call again. A later call never
    /// reallocates and never rebinds. The commit ring has no key of its own
    /// and takes `ring_out_records`: one thread drains it into every
    /// connection's ring, so it needs no more room than one of them.
    ///
    /// The swap is one pointer store, so a reader sees the old
    /// configuration or the new one and never a mix. The read lock is held
    /// by no reader for longer than a pointer copy, so nothing on a reader
    /// thread waits on the logic thread through it. The two counts are
    /// stored beside the swap rather than inside it: they are read by
    /// nothing that decides, only reported.
    pub fn configure<S, I>(&self, table: I) -> Result<Applied, ConfigureError>
    where
        S: AsRef<str>,
        I: IntoIterator<Item = (S, Value)>,
    {
        let mut configured = self
            .configuring
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let applied = if *configured {
            self.config().apply(table)?
        } else {
            let applied = Config::first(table)?;
            let addr = SocketAddr::new(applied.config.bind_address, applied.config.port);
            let ring = applied.config.ring_out_records as usize;
            self.start_outbound(addr, ring, ring)
                .map_err(|error| ConfigureError::Bind { addr, error })?;
            applied
        };
        self.pending_restart
            .store(applied.pending.len() as u64, Ordering::Relaxed);
        self.unknown_keys
            .store(applied.unknown.len() as u64, Ordering::Relaxed);
        self.swap(applied.config.clone());
        *configured = true;
        Ok(applied)
    }

    /// Put `next` in force. Called with `configuring` held.
    fn swap(&self, next: Config) {
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
    }

    /// Whether the first `configure` has happened.
    pub fn configured(&self) -> bool {
        *self
            .configuring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `config_keys_pending_restart`, as of the last `configure`.
    pub fn pending_restart(&self) -> u64 {
        self.pending_restart.load(Ordering::Relaxed)
    }

    /// Keys the last `configure` carried that the broker does not own.
    pub fn unknown_keys(&self) -> u64 {
        self.unknown_keys.load(Ordering::Relaxed)
    }

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
    /// over, and present in every handshake after that.
    pub fn handshake(&self) -> handshake::Handshake {
        handshake::Handshake {
            protocol: crate::PROTOCOL_VERSION,
            broker: crate::BROKER_VERSION,
            instance_id: self.instance_id,
            schema_sha256: self.schema.get().map(|held| held.sha256),
        }
    }

    /// Take the schema the hook driver read from its deployment, hash it,
    /// and serve it from now on: `GetSchema` answers with the bytes and the
    /// handshake carries the hash. Returns the hash.
    ///
    /// Refused before the first `configure`, because the hook driver's
    /// start is `configure` then `schema` and a call out of that order is
    /// a hook driver defect. Refused when empty, because `schema.pb` is
    /// never empty and an empty read is a file that was not found. Refused
    /// when the answer would outgrow `max_frame_bytes` as in force at the
    /// call, because the reader refuses an inbound frame over the cap and a
    /// consumer built the same way would refuse the answer; a cap lowered
    /// under the set later is the operator's, and the set stays held.
    /// Refused once a schema is held: replacing the served set is a DCS
    /// restart, so the second call changes nothing. The bytes are not
    /// parsed; the broker holds no schema it understands.
    pub fn hold_schema(&self, bytes: &[u8]) -> Result<[u8; 32], SchemaError> {
        use sha2::{Digest, Sha256};
        if !self.configured() {
            return Err(SchemaError::NotConfigured);
        }
        if bytes.is_empty() {
            return Err(SchemaError::Empty);
        }
        let max_frame_bytes = self.config().max_frame_bytes;
        if bytes.len().saturating_add(crate::inbound::ANSWER_BYTES) > max_frame_bytes as usize {
            return Err(SchemaError::TooLarge {
                len: bytes.len(),
                max_frame_bytes,
            });
        }
        let held = HeldSchema {
            set: Record::from(bytes),
            sha256: Sha256::digest(bytes).into(),
        };
        let sha256 = held.sha256;
        self.schema.set(held).map_err(|_| SchemaError::Held)?;
        Ok(sha256)
    }

    /// The schema as handed over, or `None` until the hand-off.
    pub fn schema(&self) -> Option<Record> {
        self.schema.get().map(|held| Arc::clone(&held.set))
    }

    /// Milliseconds since the process started.
    ///
    /// Saturating at the top of a u64 is 584 million years of uptime, and
    /// one below it leaves room for the heartbeat's bias.
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX - 1)
    }

    /// `shim.tick`: the logic thread is running now, and this is the sim's
    /// clock.
    ///
    /// Called every frame from the hook state. The mission time is published
    /// on every call, and the heartbeat is stamped at most once per
    /// `heartbeat_interval_ms`. The throttle is here rather than at the call
    /// site so the hook driver cannot skip the call without also skipping
    /// the mission time. Two atomic loads and at most two stores; the one
    /// caller is the logic thread, so nothing races the read of the last
    /// stamp against its write.
    pub fn tick(&self, mission_time: f64) {
        self.tick_at(self.now_ms(), mission_time);
    }

    /// `tick` with the clock supplied, so a test drives the throttle.
    fn tick_at(&self, now_ms: u64, mission_time: f64) {
        self.mission_time
            .store(mission_time.to_bits(), Ordering::Relaxed);
        let interval = self.config().heartbeat_interval_ms;
        let due = match self.heartbeat.load(Ordering::Relaxed) {
            0 => true,
            stamped => now_ms.saturating_sub(stamped - 1) >= interval,
        };
        if due {
            self.heartbeat.store(now_ms + 1, Ordering::Relaxed);
        }
    }

    /// The mission time the last `tick` published, or zero before the
    /// first.
    pub fn mission_time(&self) -> f64 {
        f64::from_bits(self.mission_time.load(Ordering::Relaxed))
    }

    /// Mark a mission load as in progress, or over.
    ///
    /// A load is a frame blackout of tens of seconds, so between the two
    /// marks `dcs_alive` is judged against `dcs_alive_threshold_loading_ms`
    /// rather than the running threshold, and a normal load does not read
    /// as a dead sim. The marks are the hook driver's `MissionLoadBegan` and
    /// `MissionLoaded`, and nothing sets this until those records exist.
    pub fn set_loading(&self, loading: bool) {
        self.loading.store(loading, Ordering::Relaxed);
    }

    /// What a `Pong` carries now: the heartbeat's age, whether that is
    /// under the threshold in force, and the kill switch's effective value.
    /// Read on the reader thread, and it touches nothing the logic thread
    /// holds.
    pub fn liveness(&self) -> Liveness {
        let now = self.now_ms();
        let last_heard_ms = match self.heartbeat.load(Ordering::Relaxed) {
            0 => None,
            stamped => Some(now.saturating_sub(stamped - 1)),
        };
        let config = self.config();
        let threshold = if self.loading.load(Ordering::Relaxed) {
            config.dcs_alive_threshold_loading_ms
        } else {
            config.dcs_alive_threshold_ms
        };
        Liveness {
            last_heard_ms,
            alive: last_heard_ms.is_some_and(|age| age < threshold),
            enabled: config.enabled,
        }
    }

    /// The effective value of the `enabled` key.
    pub fn enabled(&self) -> bool {
        self.config().enabled
    }

    /// Set the kill switch: the one key `SetEnabled` moves between two
    /// `configure`s, swapped in the same way. What a disabled bridge stops
    /// is the hook driver's to stop, and it reads this to know.
    pub fn set_enabled(&self, enabled: bool) {
        let _configuring = self
            .configuring
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = (*self.config()).clone();
        next.enabled = enabled;
        self.swap(next);
    }

    /// A consumer reported `seq` as durably processed. Counted; the number
    /// itself waits for the replay spool.
    pub fn seq_ack(&self, _seq: u64) {
        self.seq_acks.fetch_add(1, Ordering::Relaxed);
    }

    /// How many `SeqAck` records have been consumed.
    pub fn seq_acks(&self) -> u64 {
        self.seq_acks.load(Ordering::Relaxed)
    }

    /// A message on `topic` was refused because the session's token lacks
    /// the capability it requires. Counted; the `Rejected` that would tell
    /// the sender is a later task's.
    pub fn refused_no_capability(&self, _topic: &str) {
        self.no_capability.fetch_add(1, Ordering::Relaxed);
    }

    /// How many messages were refused for a capability the token lacked.
    pub fn no_capability(&self) -> u64 {
        self.no_capability.load(Ordering::Relaxed)
    }

    /// Replace the token table whole, leaving every other key as it is.
    ///
    /// The `tokens` key reaches the broker through `configure`; this is
    /// how a test hands the shared bridge one credential without touching
    /// the rest. The swap is the same one a `configure` makes, so a
    /// connection authenticates against whatever the table holds at that
    /// moment. It does not count as the first `configure`, so a first
    /// `configure` after it starts from the defaults and the table's own
    /// `tokens`, as the file is the whole configuration.
    pub fn set_tokens(&self, tokens: Vec<Token>) {
        let _configuring = self
            .configuring
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = (*self.config()).clone();
        next.tokens = tokens;
        self.swap(next);
    }

    /// Match `secret` against every token and open a session on the one
    /// that carries it.
    ///
    /// Every token is compared whether or not an earlier one matched, so
    /// the time taken says nothing about which entry, if any, was right.
    /// A match with an empty capability set is refused: the token can do
    /// nothing, which is a configuration mistake and worth a distinct error.
    /// A match past `max_connections` authenticated sessions is refused as
    /// full, and the count is taken here so two racing `Auth`s cannot both
    /// take the last slot.
    pub fn authenticate(&self, secret: &[u8]) -> Result<Session, AuthError> {
        let config = self.config();
        let mut matched = None;
        for token in config.tokens.iter() {
            if same_secret(secret, &token.secret) && matched.is_none() {
                matched = Some(token);
            }
        }
        let token = matched.ok_or(AuthError::BadToken)?;
        if token.caps.is_empty() {
            return Err(AuthError::EmptyCapabilitySet);
        }

        let limit = u64::from(config.max_connections);
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
    ///
    /// The count never goes below zero: a call with no session behind it
    /// would otherwise wrap the counter and refuse every consumer as
    /// `SERVER_FULL` until the process restarted, which is too large a
    /// blast radius for one misplaced call.
    pub fn disconnected(&self, _session: &Session) {
        let mut held = self.authenticated.load(Ordering::Relaxed);
        while held > 0 {
            match self.authenticated.compare_exchange_weak(
                held,
                held - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(now) => held = now,
            }
        }
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
            "/../../proto/dcsbridge/broker/broker.proto"
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
    }

    /// A secret no token carries is a bad token, a token granting nothing
    /// is refused as such, the count of sessions is held under the cap and
    /// comes back down at disconnect, and a secret of the wrong length is
    /// wrong.
    #[test]
    fn authentication_matches_a_token_and_holds_the_session_count() {
        let bridge = Bridge::new(1);
        let max_connections = Config::default().max_connections;
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

        let sessions: Vec<Session> = (0..max_connections)
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
        // A disconnect with no session behind it leaves the count at zero
        // rather than wrapping it into a permanent `SERVER_FULL`.
        for session in &sessions {
            bridge.disconnected(session);
        }
        bridge.disconnected(&sessions[0]);
        assert_eq!(bridge.authenticated(), 0, "the count went below zero");
        assert!(bridge.authenticate(b"correct horse").is_ok());

        assert!(same_secret(b"", b""));
        assert!(!same_secret(b"a", b""));
        assert!(!same_secret(b"", b"a"));
    }

    /// A `configure` is one swap: the token table and the connection cap
    /// bind on the next `Auth`, the kill switch and the alive threshold on
    /// the next `Ping`, the reader's limits on its next read. A later call
    /// applies the live keys, counts a changed restart-tier key and an
    /// unknown one, and a refused call changes nothing at all.
    #[test]
    fn configure_swaps_the_live_keys_and_counts_the_rest() {
        let bridge = Bridge::new(2);
        let n = Value::Number;
        let token = |id: &str, secret: &[u8]| Token {
            id: id.into(),
            secret: secret.to_vec(),
            caps: [Capability::Read].into_iter().collect(),
        };
        assert!(!bridge.configured());
        assert!(bridge.liveness().enabled);

        // The first call: every tier applies, including the cap, and the
        // listener binds on the port the table names.
        let applied = bridge
            .configure([
                ("port", n(0.0)),
                ("max_connections", n(2.0)),
                ("max_unauthenticated_connections", n(1.0)),
                ("handshake_timeout_ms", n(250.0)),
                ("enabled", Value::Boolean(false)),
                ("tokens", Value::Tokens(vec![token("a", b"first")])),
                ("route", Value::String("A".into())),
            ])
            .expect("a valid table");
        assert!(bridge.configured());
        assert_eq!((applied.live, applied.pending.len()), (4, 0));
        assert_eq!(bridge.unknown_keys(), 1);
        assert_eq!(bridge.pending_restart(), 0);
        assert!(!bridge.liveness().enabled, "enabled did not apply");
        assert_eq!(
            Limits::from(&*bridge.config()).handshake_timeout,
            std::time::Duration::from_millis(250)
        );
        let kept = bridge.authenticate(b"first").expect("the new token");
        bridge.authenticate(b"first").expect("under the cap of two");
        assert_eq!(bridge.authenticate(b"first"), Err(AuthError::ServerFull));

        // A later call: the token table and the switch move, the cap does
        // not and is counted, and a live key the table drops reverts.
        let applied = bridge
            .configure([
                ("max_connections", n(8.0)),
                ("max_unauthenticated_connections", n(1.0)),
                ("enabled", Value::Boolean(true)),
                ("tokens", Value::Tokens(vec![token("b", b"second")])),
            ])
            .expect("a valid table");
        assert_eq!(applied.pending[0].key, "max_connections");
        assert_eq!(bridge.pending_restart(), 1);
        assert_eq!(bridge.unknown_keys(), 0);
        assert!(bridge.liveness().enabled);
        assert_eq!(
            Limits::from(&*bridge.config()).handshake_timeout,
            Limits::default().handshake_timeout,
            "a dropped live key did not revert"
        );
        assert_eq!(bridge.authenticate(b"first"), Err(AuthError::BadToken));
        assert_eq!(
            bridge.authenticate(b"second"),
            Err(AuthError::ServerFull),
            "the cap moved without a restart"
        );
        bridge.disconnected(&kept);
        bridge.authenticate(b"second").expect("the freed slot");

        // A refused call, on a value and on an invariant, leaves the
        // configuration in force as it was, counts included.
        let before = bridge.config();
        assert!(
            bridge
                .configure([("enabled", Value::Boolean(false)), ("port", n(-1.0))])
                .is_err()
        );
        assert!(
            bridge
                .configure([("max_unauthenticated_connections", n(2.0))])
                .is_err(),
            "the effective cap is two"
        );
        assert_eq!(*bridge.config(), *before);
        assert!(bridge.liveness().enabled);
        assert_eq!(bridge.pending_restart(), 1);
    }

    /// The schema is refused before the first `configure` and when empty,
    /// held once with its hash in every handshake after, handed to the
    /// reader byte for byte, and refused a second time with the first still
    /// held.
    #[test]
    fn the_schema_is_held_once_after_configure_and_served_back() {
        use sha2::{Digest, Sha256};

        let bridge = Bridge::new(3);
        let set = b"\x0a\x05hello".to_vec();
        assert_eq!(
            bridge.hold_schema(&set),
            Err(SchemaError::NotConfigured),
            "held before configure"
        );
        assert!(bridge.schema().is_none());
        assert_eq!(bridge.handshake().schema_sha256, None);

        bridge
            .configure([("port", Value::Number(0.0))])
            .expect("a valid table");
        assert_eq!(bridge.hold_schema(b""), Err(SchemaError::Empty));
        assert!(bridge.schema().is_none(), "an empty schema was held");

        // A set whose answer would not fit the frame cap in force is
        // refused naming both, and the cap is the live key as of the call.
        bridge
            .configure([("max_frame_bytes", Value::Number(256.0))])
            .expect("a valid table");
        let wide = vec![0x0a; 200];
        assert_eq!(
            bridge.hold_schema(&wide),
            Err(SchemaError::TooLarge {
                len: 200,
                max_frame_bytes: 256,
            })
        );
        assert!(bridge.schema().is_none(), "an oversized schema was held");
        bridge
            .configure([("max_frame_bytes", Value::Number(1024.0))])
            .expect("a valid table");

        let sha256 = bridge.hold_schema(&set).expect("the first hand-off");
        assert_eq!(sha256, <[u8; 32]>::from(Sha256::digest(&set)));
        assert_eq!(bridge.handshake().schema_sha256, Some(sha256));

        let held = bridge.schema().expect("a schema to serve");
        assert_eq!(
            &held[..],
            &set[..],
            "the held set is not the one handed over"
        );

        assert_eq!(
            bridge.hold_schema(b"\x0a\x05other"),
            Err(SchemaError::Held),
            "a second hand-off was applied"
        );
        assert_eq!(bridge.handshake().schema_sha256, Some(sha256));
        assert!(
            Arc::ptr_eq(&held, &bridge.schema().unwrap()),
            "the held set changed"
        );
    }

    /// Before any registration the acknowledgement is the one addressable
    /// topic, a fan-out topic is refused, and only the refusal is counted.
    #[test]
    fn the_acknowledgement_is_addressable_and_a_fan_out_topic_is_refused() {
        let before = bridge().misaddressed();

        assert!(bridge().addressable(dcsbridge_topic::COMMAND_ACK.as_bytes()));
        assert_eq!(
            bridge().misaddressed(),
            before,
            "an accepted address was counted"
        );

        // A fan-out topic, one the broker does not know by name.
        const FANOUT: &[u8] = b"dcsbridge.builtin.sim.UnitDestroyed";
        assert!(!bridge().addressable(FANOUT));
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
        use std::io::{Read, Write};
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

        // Nothing fans out until the connection authenticates against the
        // bridge's own token table, and the result is frame two.
        bridge().set_tokens(vec![Token {
            id: "test".into(),
            secret: b"hunter2".to_vec(),
            caps: [Capability::Read].into_iter().collect(),
        }]);
        let auth = {
            use prost::Message;
            let body = crate::inbound::Envelope {
                seq: 1,
                payload: Some(crate::inbound::Payload {
                    type_url: format!(
                        "{}{}",
                        dcsbridge_topic::TYPE_URL_PREFIX,
                        dcsbridge_topic::AUTH
                    ),
                    value: crate::inbound::Auth {
                        token: "hunter2".into(),
                    }
                    .encode_to_vec(),
                }),
            }
            .encode_to_vec();
            let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
            bytes.extend(body);
            bytes
        };
        client.write_all(&auth).expect("the auth is sent");
        client.read_exact(&mut length).expect("the result arrives");
        let mut frame = vec![0u8; u32::from_le_bytes(length) as usize];
        client
            .read_exact(&mut frame)
            .expect("the result's body arrives");
        assert_eq!(&frame[..2], [0x08, 0x02], "the auth result is not seq 2");
        assert_eq!(
            &frame[2..],
            &crate::inbound::auth_result(Ok(()))[..],
            "the auth did not succeed"
        );
        assert_eq!(bridge().authenticated(), 1);

        // Authenticated is queued after the result, so a record committed
        // now reaches the connection, numbered after it.
        let tail = [0x22, 0x00];
        bridge().commit(&tail).expect("the path is started");
        client.read_exact(&mut length).expect("a frame arrives");
        assert_eq!(u32::from_le_bytes(length), 2 + tail.len() as u32);
        let mut frame = vec![0u8; u32::from_le_bytes(length) as usize];
        client
            .read_exact(&mut frame)
            .expect("the frame's body arrives");
        assert_eq!(frame, [0x08, 0x03, 0x22, 0x00], "seq 3 then the tail");
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

    /// A bridge with `heartbeat_interval_ms` at `interval` and nothing
    /// started: the throttle reads the key and nothing else.
    fn ticking(instance_id: u64, interval: u64) -> Bridge {
        let bridge = Bridge::new(instance_id);
        bridge.swap(Config {
            heartbeat_interval_ms: interval,
            ..Config::default()
        });
        bridge
    }

    /// The clock reading the heartbeat was last stamped at, without the
    /// bias that makes zero mean never.
    fn stamped_at(bridge: &Bridge) -> Option<u64> {
        match bridge.heartbeat.load(Ordering::Relaxed) {
            0 => None,
            stamped => Some(stamped - 1),
        }
    }

    /// The first tick stamps whatever the clock reads, and every tick
    /// publishes its mission time whether or not it stamps.
    #[test]
    fn the_first_tick_stamps_and_every_tick_publishes_mission_time() {
        let bridge = ticking(10, 1000);
        assert_eq!(stamped_at(&bridge), None);
        assert_eq!(bridge.liveness().last_heard_ms, None);

        bridge.tick_at(5, 12.5);
        assert_eq!(stamped_at(&bridge), Some(5));
        assert_eq!(bridge.mission_time(), 12.5);

        bridge.tick_at(6, 12.75);
        assert_eq!(
            stamped_at(&bridge),
            Some(5),
            "a tick inside the interval stamped"
        );
        assert_eq!(
            bridge.mission_time(),
            12.75,
            "the mission time was throttled"
        );
    }

    /// Ticks inside one interval stamp once; the first at or past the
    /// interval stamps again, and the interval is measured from the last
    /// stamp rather than the last tick.
    #[test]
    fn the_heartbeat_is_stamped_at_most_once_per_interval() {
        let bridge = ticking(11, 1000);

        bridge.tick_at(100, 0.0);
        for now in (116..1100).step_by(16) {
            bridge.tick_at(now, 0.0);
        }
        assert_eq!(
            stamped_at(&bridge),
            Some(100),
            "a tick under the interval stamped"
        );

        bridge.tick_at(1100, 0.0);
        assert_eq!(
            stamped_at(&bridge),
            Some(1100),
            "the tick at the interval did not stamp"
        );

        bridge.tick_at(2000, 0.0);
        assert_eq!(stamped_at(&bridge), Some(1100));
        bridge.tick_at(2100, 0.0);
        assert_eq!(stamped_at(&bridge), Some(2100));
    }

    /// The interval is the key in force, read at each tick.
    #[test]
    fn the_throttle_reads_the_interval_in_force() {
        let bridge = ticking(12, 100);

        bridge.tick_at(0, 0.0);
        bridge.tick_at(100, 0.0);
        assert_eq!(stamped_at(&bridge), Some(100));

        bridge.swap(Config {
            heartbeat_interval_ms: 500,
            ..Config::default()
        });
        bridge.tick_at(200, 0.0);
        assert_eq!(stamped_at(&bridge), Some(100), "the old interval was read");
        bridge.tick_at(600, 0.0);
        assert_eq!(stamped_at(&bridge), Some(600));
    }

    /// A stamp older than the running threshold reads dead while running
    /// and alive while a load is in progress, and the load's end puts the
    /// running threshold back. The stamp is real time here, so the two
    /// thresholds straddle an age of zero rather than waiting one out.
    #[test]
    fn a_load_in_progress_judges_liveness_by_the_loading_threshold() {
        let bridge = Bridge::new(13);
        bridge.swap(Config {
            heartbeat_interval_ms: 0,
            dcs_alive_threshold_ms: 0,
            dcs_alive_threshold_loading_ms: 120_000,
            ..Config::default()
        });
        assert!(!bridge.liveness().alive, "never heard from read alive");

        bridge.tick(0.0);
        assert!(
            !bridge.liveness().alive,
            "an age at the running threshold read alive"
        );

        bridge.set_loading(true);
        assert!(
            bridge.liveness().alive,
            "a load in progress read the running threshold"
        );

        bridge.set_loading(false);
        assert!(
            !bridge.liveness().alive,
            "the load's end kept the loading threshold"
        );
    }
}
