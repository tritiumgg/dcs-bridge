//! The state one DCS process shares, and what a module open does to it.
//!
//! Both DCS Lua states load the broker, so `luaopen_dcsbridge` runs more than
//! once and each state gets its own table. Behind those tables is one
//! [`Bridge`]. It has to be one: the hook driver and the sim driver register
//! from different states, and a per-state map would leave each registrar blind
//! to what the other had done.
//!
//! `Bridge` is where everything process-global lives. The registration maps
//! of [`crate::registry`] are here, and the outbound path, the writer thread
//! over the commit ring and the listener that fans it out, joins them once
//! [`Bridge::start_outbound`] is called. The two inbound rings, one per
//! target, join them at [`Bridge::start_inbound`]: every connection's reader
//! thread pushes into the one its route map names, and Lua polls each from
//! the state that owns it. ADR 0007, ADR 0024.
//!
//! Both DCS states commit records, and they run on one thread, so the commit
//! ring's one producer is shared between them behind a lock that is never
//! waited on: `try_lock`, with contention refused and counted, because a
//! second thread committing is a defect rather than a case. ADR 0014.

use std::collections::HashSet;
use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
    TryLockError,
};
use std::time::Instant;

use crate::config::{self, Applied, Config, Value};
use crate::encode::Stamp;
use crate::fanout::{Commit, ConnectionId, Writer};
use crate::handshake;
use crate::inbound::{
    Answers, AuthError, Command, Delivery, Limits, Liveness, RejectedReason, Session, Window,
};
use crate::registry::{Capability, Conflict, Member, RecordClass, Registry, Target, Topic};
use crate::ring::{Consumer, Producer, Push, Ring};
use crate::transport::{Listener, Record};

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
    inbound: OnceLock<Inbound>,
    /// Held while the outbound path or the inbound rings are being started,
    /// so two starters cannot both bind or both allocate.
    starting: Mutex<()>,
    /// Inbound records whose topic no route map names, dropped and counted.
    /// `unrouted_topic_total`.
    unrouted: AtomicU64,
    /// Records refused at `begin_to` because their topic is neither a reply
    /// nor the acknowledgement.
    misaddressed: AtomicU64,
    /// Records refused at `begin` or `begin_to` because their topic has no
    /// class or no capability registered. `partial_registration_total`.
    partial_registration: AtomicU64,
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
    /// The epoch `shim.epoch` opened, or zero between epochs. The hook
    /// driver allocates ids from one, so zero is free to mean none.
    epoch: AtomicU32,
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
    /// Records refused, by reason, in `RejectedReason::ALL`'s order:
    /// `commands_rejected_total`. Every refusal counts here, answered or
    /// not.
    commands_rejected: [AtomicU64; 4],
    /// Refusals above `rejected_max_per_sec` or `busy_max_per_sec`,
    /// counted here and answered with nothing: `rejections_suppressed_total`.
    rejections_suppressed: AtomicU64,
    /// What every connection together has sent for Lua this second,
    /// against `inbound_records_per_sec_total`. Reader threads take the
    /// lock, for one count each; the logic thread never does. ADR 0026.
    inbound_total: Mutex<Window>,
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
            .field("filtered", &self.writer.filtered())
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

    /// How many times a fanned-out record was withheld from a connection
    /// whose token did not cover it, `records_filtered_total`.
    pub fn filtered(&self) -> u64 {
        self.writer.filtered()
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

/// The inbound rings: one per target, each with the reader threads at one
/// end and a Lua state at the other.
///
/// The producer is shared by every connection's reader thread behind a
/// lock those threads alone take, for one push each; the consumer is the
/// logic thread's and is taken the way the commit ring's producer is, with
/// a `try_lock` that refuses rather than waits. ADR 0024.
pub struct Inbound {
    sim: Lane,
    hook: Lane,
}

impl Inbound {
    /// The ring `target` polls.
    fn lane(&self, target: Target) -> &Lane {
        match target {
            Target::SimDriver => &self.sim,
            Target::HookDriver => &self.hook,
        }
    }
}

impl fmt::Debug for Inbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inbound")
            .field("sim_busy", &self.sim.busy.load(Ordering::Relaxed))
            .field("hook_busy", &self.hook.busy.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// One inbound ring's two ends and its count of records turned away.
struct Lane {
    producer: Mutex<Producer<Command>>,
    consumer: Mutex<Consumer<Command>>,
    /// Records the ring had no room for. Each was the newest at the time
    /// and came back to the reader thread that offered it.
    busy: AtomicU64,
}

impl Lane {
    fn new(capacity: usize) -> Self {
        let (producer, consumer) = Ring::split(capacity);
        Lane {
            producer: Mutex::new(producer),
            consumer: Mutex::new(consumer),
            busy: AtomicU64::new(0),
        }
    }
}

/// Why a `poll` answered nothing at all, rather than an empty ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollError {
    /// The inbound rings have not been allocated: `configure` comes first.
    NotStarted,
    /// Another thread was polling the same ring. Refused, because waiting
    /// would put a lock on the logic thread and a second poller is a
    /// defect.
    Busy,
}

impl fmt::Display for PollError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PollError::NotStarted => "configure comes first: the rings are allocated by it",
            PollError::Busy => "another thread is polling this ring",
        })
    }
}

impl std::error::Error for PollError {}

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

    fn rejected(&self, reason: RejectedReason, answered: bool) {
        bridge().rejected(reason, answered);
    }

    fn admit_total(&self, now: Instant, cap: u32) -> bool {
        bridge().admit_total(now, cap)
    }

    fn deliver(&self, caps: &HashSet<Capability>, command: Command) -> Delivery {
        bridge().deliver(caps, command)
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
            inbound: OnceLock::new(),
            starting: Mutex::new(()),
            unrouted: AtomicU64::new(0),
            misaddressed: AtomicU64::new(0),
            partial_registration: AtomicU64::new(0),
            instance_id,
            started: Instant::now(),
            heartbeat: AtomicU64::new(0),
            mission_time: AtomicU64::new(0),
            loading: AtomicBool::new(false),
            epoch: AtomicU32::new(0),
            config: RwLock::new(Arc::new(Config::default())),
            configuring: Mutex::new(false),
            pending_restart: AtomicU64::new(0),
            unknown_keys: AtomicU64::new(0),
            authenticated: AtomicU64::new(0),
            seq_acks: AtomicU64::new(0),
            commands_rejected: [const { AtomicU64::new(0) }; 4],
            rejections_suppressed: AtomicU64::new(0),
            inbound_total: Mutex::new(Window::new(Instant::now())),
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
    /// The first call is also what allocates and binds: the two inbound
    /// rings, the commit ring, the listener on `bind_address` and `port`,
    /// and the ring each connection gets, all sized from the table. The
    /// inbound rings come first, so no connection can be accepted before
    /// the ring its records go to exists. A bind that fails refuses the
    /// call with nothing in force and nothing started, so the hook driver
    /// can fix the address and call again; the inbound rings it allocated
    /// stay, sized as the refused table sized them, since the next call
    /// carries the same restart-tier keys or the file has changed under a
    /// broker that has never listened. A later call never reallocates and
    /// never rebinds. The commit ring has no key of its own and takes
    /// `ring_out_records`: one thread drains it into every connection's
    /// ring, so it needs no more room than one of them.
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
            self.start_inbound(
                applied.config.ring_in_sim_driver_records as usize,
                applied.config.ring_in_hook_driver_records as usize,
            );
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

    /// Allocate the two inbound rings, `sim_capacity` records for the sim
    /// driver's and `hook_capacity` for the hook driver's, once. A second
    /// call changes nothing.
    ///
    /// # Panics
    ///
    /// If either capacity is zero, which the configuration's check on the
    /// two keys does not let through.
    pub fn start_inbound(&self, sim_capacity: usize, hook_capacity: usize) {
        let _starting = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        if self.inbound.get().is_some() {
            return;
        }
        // The lock above makes this the only setter.
        let _ = self.inbound.set(Inbound {
            sim: Lane::new(sim_capacity),
            hook: Lane::new(hook_capacity),
        });
    }

    /// The inbound rings, once allocated.
    pub fn inbound(&self) -> Option<&Inbound> {
        self.inbound.get()
    }

    /// Put an inbound record from a session holding `caps` on the ring its
    /// route names, or hand it back.
    ///
    /// Called on a reader thread. The route and the capability are read
    /// under the registry's read lock and the lock released before the
    /// ring's producer is taken, so no thread holds both. A topic in no
    /// route map goes nowhere and is counted, never defaulted to the sim
    /// driver: a command the sim driver was not told to expect is not one
    /// it should run. A routed topic the session's token does not cover
    /// goes nowhere either, and so does one with no capability registered:
    /// nothing says the token covers it, so the record fails closed the
    /// way a `begin` on such a topic does. A full ring turns the newest
    /// record away and counts it; the reader thread answers the sender
    /// with `Rejected` in every case. Before the rings exist nothing can
    /// have connected, so a record arriving then is a defect, and it is
    /// dropped and counted as unrouted rather than held anywhere. ADR 0024.
    pub fn deliver(&self, caps: &HashSet<Capability>, command: Command) -> Delivery {
        let (target, covered) = {
            let registry = self.registry();
            let target = registry.routes().get(&command.topic).copied();
            let covered = registry
                .caps()
                .get(&command.topic)
                .is_some_and(|required| caps.contains(required));
            (target, covered)
        };
        let lane = match (target, self.inbound.get()) {
            (Some(target), Some(inbound)) => inbound.lane(target),
            _ => {
                self.unrouted.fetch_add(1, Ordering::Relaxed);
                return Delivery::Unrouted(command);
            }
        };
        if !covered {
            return Delivery::Uncovered(command);
        }
        let offered = lane
            .producer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .offer(command);
        match offered {
            Push::Stored => Delivery::Stored,
            Push::Refused(command) | Push::Evicted(command) => {
                lane.busy.fetch_add(1, Ordering::Relaxed);
                Delivery::Busy(command)
            }
        }
    }

    /// Take the oldest record from `target`'s ring, or `None` when it holds
    /// nothing.
    ///
    /// Called on the logic thread, by the Lua state the target names. The
    /// consumer is never waited on: a second thread polling the same ring
    /// is a defect, so it is refused and reported rather than served.
    pub fn poll(&self, target: Target) -> Result<Option<Command>, PollError> {
        let lane = self
            .inbound
            .get()
            .ok_or(PollError::NotStarted)?
            .lane(target);
        match lane.consumer.try_lock() {
            Ok(mut consumer) => Ok(consumer.pop()),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner().pop()),
            Err(TryLockError::WouldBlock) => Err(PollError::Busy),
        }
    }

    /// How many inbound records named a topic in no route map, and were
    /// dropped for it.
    pub fn unrouted_topic(&self) -> u64 {
        self.unrouted.load(Ordering::Relaxed)
    }

    /// How many inbound records `target`'s ring had no room for. Zero
    /// before the rings exist.
    pub fn inbound_busy(&self, target: Target) -> u64 {
        self.inbound.get().map_or(0, |inbound| {
            inbound.lane(target).busy.load(Ordering::Relaxed)
        })
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

    /// `shim.epoch`: open an epoch, or close the one that is open.
    ///
    /// The hook driver publishes the id once per boundary, at mission load
    /// end and at simulation stop, and every record committed between the
    /// two carries it. One store; the id is never zero, because zero is
    /// what the field reads as before it is set.
    pub fn set_epoch(&self, epoch: Option<u32>) {
        self.epoch.store(epoch.unwrap_or(0), Ordering::Relaxed);
    }

    /// What `begin` writes into the envelope: the open epoch and the clock,
    /// or `None` outside an epoch, when a record carries neither.
    ///
    /// Read on the logic thread, the same thread that publishes both, so
    /// the pair is the frame's own.
    pub fn stamp(&self) -> Option<Stamp> {
        match self.epoch.load(Ordering::Relaxed) {
            0 => None,
            epoch => Some(Stamp {
                epoch,
                mission_time: self.mission_time(),
            }),
        }
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

    /// A record was refused for `reason`; `answered` says whether the
    /// `Rejected` was sent or withheld by its cap.
    pub fn rejected(&self, reason: RejectedReason, answered: bool) {
        self.commands_rejected[reason as usize - 1].fetch_add(1, Ordering::Relaxed);
        if !answered {
            self.rejections_suppressed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// How many records were refused for `reason`, answered or not.
    pub fn commands_rejected(&self, reason: RejectedReason) -> u64 {
        self.commands_rejected[reason as usize - 1].load(Ordering::Relaxed)
    }

    /// How many refusals were over their cap and answered with nothing.
    pub fn rejections_suppressed(&self) -> u64 {
        self.rejections_suppressed.load(Ordering::Relaxed)
    }

    /// Count one record for Lua at `now` against every connection's total,
    /// and say whether it fit under `cap`. Called on a reader thread.
    pub fn admit_total(&self, now: Instant, cap: u32) -> bool {
        self.inbound_total
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(now, cap)
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

    /// Queue an envelope tail for every connection whose token covers
    /// `need`, the capability its topic requires.
    ///
    /// The tail is copied once, into the allocation the rings share by
    /// reference; that is the one allocation on the commit path. A record the
    /// commit ring evicts to make room comes back here and is dropped on the
    /// calling thread. ADR 0014.
    pub fn commit(&self, need: Capability, tail: &[u8]) -> Result<(), CommitError> {
        let outbound = self.outbound.get().ok_or(CommitError::NotStarted)?;
        let record: Record = Arc::from(tail);

        let mut commit = outbound.producer()?;
        drop(commit.push(need.number(), record));
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

    /// The capability `topic` requires, or `None` when it has no class or no
    /// capability registered, counting a refusal in
    /// `partial_registration_total`.
    ///
    /// Asked at every `begin` and `begin_to`, and answered the way
    /// [`Bridge::addressable`] is: a plain value with no lock held, so the
    /// raise the Lua side makes of `None` jumps past no guard. The value
    /// travels with the record to [`Bridge::commit`], where the writer
    /// thread withholds the record from a connection it does not cover.
    pub fn registered(&self, topic: &[u8]) -> Option<Capability> {
        let required = self.registry().required(topic);
        if required.is_none() {
            self.partial_registration.fetch_add(1, Ordering::Relaxed);
        }
        required
    }

    /// How many records were refused at `begin` or `begin_to` for naming a
    /// topic missing a class or a capability. A topic in that state comes
    /// from generated files of two different runs, and `doctor` names each
    /// such topic.
    pub fn partial_registration(&self) -> u64 {
        self.partial_registration.load(Ordering::Relaxed)
    }

    /// Write to the registration maps.
    ///
    /// Reads through poison for the reason [`Bridge::registry`] does. The
    /// only writers are the two registrars, both on the logic thread, so
    /// the lock is never waited on by them; a reader on another thread
    /// waits for the length of one merge.
    fn registry_mut(&self) -> RwLockWriteGuard<'_, Registry> {
        self.registry
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Merge a table of drop policies, answering the rows it added, or the
    /// conflict that refused it whole. [`Registry::register_classes`].
    pub fn register_classes(
        &self,
        rows: impl IntoIterator<Item = (Topic, RecordClass)>,
    ) -> Result<usize, Conflict> {
        self.registry_mut().register_classes(rows)
    }

    /// Merge a table of destination states. [`Registry::register_routes`].
    pub fn register_routes(
        &self,
        rows: impl IntoIterator<Item = (Topic, Target)>,
    ) -> Result<usize, Conflict> {
        self.registry_mut().register_routes(rows)
    }

    /// Merge a table of required capabilities. [`Registry::register_caps`].
    pub fn register_caps(
        &self,
        rows: impl IntoIterator<Item = (Topic, Capability)>,
    ) -> Result<usize, Conflict> {
        self.registry_mut().register_caps(rows)
    }

    /// Add topics to the addressable set, answering how many were new.
    /// [`Registry::register_replies`].
    pub fn register_replies(&self, topics: impl IntoIterator<Item = Topic>) -> usize {
        self.registry_mut().register_replies(topics)
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

        let pairs: [(&str, i32); 16] = [
            (
                "REJECTED_REASON_UNKNOWN_TOPIC",
                RejectedReason::UnknownTopic as i32,
            ),
            (
                "REJECTED_REASON_NO_CAPABILITY",
                RejectedReason::NoCapability as i32,
            ),
            (
                "REJECTED_REASON_RATE_LIMITED",
                RejectedReason::RateLimited as i32,
            ),
            ("REJECTED_REASON_BUSY", RejectedReason::Busy as i32),
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

    /// A registration goes through the bridge's write lock and is read
    /// back through its read lock: two registrars over disjoint sets both
    /// merge, a repeat adds nothing, and a conflict is refused with the
    /// maps as they were.
    #[test]
    fn registrations_merge_through_the_bridge() {
        let bridge = Bridge::new(3);
        const EVENT: &str = "dcsbridge.builtin.sim.UnitDestroyed";
        const COMMAND: &str = "dcsbridge.builtin.sim.SetFlag";

        assert_eq!(
            bridge.register_classes([(EVENT.to_string(), RecordClass::Durable)]),
            Ok(1)
        );
        assert_eq!(
            bridge.register_classes([
                (EVENT.to_string(), RecordClass::Durable),
                (COMMAND.to_string(), RecordClass::Command),
            ]),
            Ok(1)
        );
        assert_eq!(
            bridge.register_routes([(COMMAND.to_string(), Target::SimDriver)]),
            Ok(1)
        );
        let refused = bridge.register_caps([
            (EVENT.to_string(), Capability::Read),
            (COMMAND.to_string(), Capability::Command),
        ]);
        assert_eq!(refused, Ok(2));
        let refused = bridge.register_caps([(EVENT.to_string(), Capability::Command)]);
        assert_eq!(
            refused.map_err(|c| c.to_string()),
            Err(format!("{EVENT} is registered as read, not command"))
        );

        let registry = bridge.registry();
        assert_eq!(registry.classes().len(), 2);
        assert_eq!(registry.routes().len(), 1);
        assert_eq!(registry.caps()[EVENT], Capability::Read);
    }

    /// A `begin` on a topic with a class and a capability is allowed, one
    /// on a topic missing either is refused and counted, and a registered
    /// reply becomes addressable.
    #[test]
    fn a_begin_needs_a_class_and_a_capability_and_a_reply_needs_registering() {
        let bridge = Bridge::new(4);
        const EVENT: &str = "dcsbridge.builtin.sim.UnitDestroyed";
        const REPLY: &str = "dcsbridge.builtin.sim.FlagValue";

        assert_eq!(bridge.registered(EVENT.as_bytes()), None);
        assert_eq!(bridge.partial_registration(), 1);

        bridge
            .register_classes([(EVENT.to_string(), RecordClass::Durable)])
            .expect("classes");
        assert_eq!(
            bridge.registered(EVENT.as_bytes()),
            None,
            "a class alone made a topic registered"
        );
        bridge
            .register_caps([(EVENT.to_string(), Capability::Read)])
            .expect("caps");
        assert_eq!(
            bridge.registered(EVENT.as_bytes()),
            Some(Capability::Read),
            "a registered topic did not name its capability"
        );
        assert_eq!(
            bridge.partial_registration(),
            2,
            "an allowed begin was counted"
        );

        assert_eq!(
            bridge.registered(dcsbridge_topic::COMMAND_ACK.as_bytes()),
            Some(Capability::Command)
        );

        assert!(!bridge.addressable(REPLY.as_bytes()));
        assert_eq!(bridge.register_replies([REPLY.to_string()]), 1);
        assert!(bridge.addressable(REPLY.as_bytes()));
        assert_eq!(bridge.misaddressed(), 1);
    }

    /// The outbound path starts once, and a record committed after that
    /// reaches a connection as a frame carrying that connection's `seq`.
    ///
    /// The bridge is the process's, so this shares it with every other test
    /// in the binary; none of the others starts the outbound path, commits,
    /// ticks, opens an epoch or reads the liveness or the stamp, and a
    /// second start returns the first's address rather than failing. A test
    /// that needs any of those takes a `Bridge::new` of its own.
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
        bridge()
            .commit(Capability::Read, &tail)
            .expect("the path is started");
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

        // A record opened the way the Lua `begin` opens one, on the stamp
        // the bridge holds at that moment, reaches the connection carrying
        // the epoch and the clock while an epoch is open, and neither once
        // it has closed. Read with a stock decoder, since the fields are the
        // consumer's to read.
        use prost::Message;
        #[derive(Clone, PartialEq, Message)]
        struct Stamped {
            #[prost(uint64, tag = "1")]
            seq: u64,
            #[prost(uint32, optional, tag = "2")]
            epoch: Option<u32>,
            #[prost(double, optional, tag = "3")]
            mission_time: Option<f64>,
        }
        let read_stamped = |client: &mut TcpStream| {
            let mut length = [0u8; 4];
            client.read_exact(&mut length).expect("a frame arrives");
            let mut frame = vec![0u8; u32::from_le_bytes(length) as usize];
            client
                .read_exact(&mut frame)
                .expect("the frame's body arrives");
            Stamped::decode(&frame[..]).expect("a stock decoder reads the envelope")
        };
        let commit_on = |topic: &[u8]| {
            let mut e = crate::encode::Encoder::with_capacity(256);
            e.begin(topic, bridge().stamp());
            e.integer(1, 1).unwrap();
            bridge()
                .commit(Capability::Read, e.commit().unwrap())
                .expect("the path is started");
        };

        bridge().set_epoch(Some(3));
        bridge().tick(12.5);
        commit_on(b"dcsbridge.builtin.sim.UnitDestroyed");
        assert_eq!(
            read_stamped(&mut client),
            Stamped {
                seq: 4,
                epoch: Some(3),
                mission_time: Some(12.5),
            }
        );

        bridge().set_epoch(None);
        commit_on(b"dcsbridge.builtin.sim.UnitDestroyed");
        assert_eq!(
            read_stamped(&mut client),
            Stamped {
                seq: 5,
                epoch: None,
                mission_time: None,
            },
            "a record outside an epoch carried a stamp"
        );

        // A record sent on a registered inbound topic the token covers
        // reaches the ring its route names, through the shared bridge's
        // own reader thread, and is polled with the sender's connection id
        // and its bytes.
        bridge().start_inbound(4, 4);
        bridge()
            .register_routes([(HOOK_COMMAND.to_string(), Target::HookDriver)])
            .expect("a new route merges");
        bridge()
            .register_caps([(HOOK_COMMAND.to_string(), Capability::Read)])
            .expect("a new capability merges");
        let sent = {
            let body = crate::inbound::Envelope {
                seq: 2,
                payload: Some(crate::inbound::Payload {
                    type_url: format!("{}{}", dcsbridge_topic::TYPE_URL_PREFIX, HOOK_COMMAND),
                    value: vec![0x08, 0x2a],
                }),
            }
            .encode_to_vec();
            let mut bytes = (body.len() as u32).to_le_bytes().to_vec();
            bytes.extend(body);
            bytes
        };
        client.write_all(&sent).expect("the command is sent");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let polled = loop {
            if let Some(command) = bridge().poll(Target::HookDriver).expect("the rings exist") {
                break command;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the command never reached the hook ring"
            );
            std::thread::yield_now();
        };
        assert_eq!(polled.from, ConnectionId::from_raw(1), "the sender's id");
        assert_eq!(polled.topic, HOOK_COMMAND, "the topic as registered");
        assert_eq!(polled.value, [0x08, 0x2a], "the payload's bytes");
        assert_eq!(
            bridge().poll(Target::SimDriver),
            Ok(None),
            "the sim ring saw it"
        );
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

    /// Outside an epoch there is no stamp, whatever the clock reads.
    /// Inside one the stamp is the epoch and the clock as last published,
    /// and closing the epoch takes the stamp away without touching the
    /// clock.
    #[test]
    fn the_stamp_is_the_open_epoch_and_the_clock_or_nothing() {
        let bridge = ticking(14, 1000);
        bridge.tick_at(0, 41.5);
        assert_eq!(bridge.stamp(), None, "a stamp outside an epoch");

        bridge.set_epoch(Some(3));
        assert_eq!(
            bridge.stamp(),
            Some(Stamp {
                epoch: 3,
                mission_time: 41.5
            })
        );

        bridge.tick_at(1, 42.0);
        assert_eq!(
            bridge.stamp().map(|stamp| stamp.mission_time),
            Some(42.0),
            "the stamp did not follow the clock"
        );

        bridge.set_epoch(None);
        assert_eq!(bridge.stamp(), None, "a stamp after the epoch closed");
        assert_eq!(
            bridge.mission_time(),
            42.0,
            "closing the epoch moved the clock"
        );
    }

    const SIM_COMMAND: &str = "dcsbridge.builtin.sim.SetFlag";
    const HOOK_COMMAND: &str = "dcsbridge.builtin.hook.Kick";

    /// A command from connection `from` on `topic`, as the reader thread
    /// hands one over.
    fn command(from: u64, topic: &str, value: &[u8]) -> Command {
        Command {
            from: ConnectionId::from_raw(from),
            topic: topic.to_owned(),
            value: value.to_vec(),
        }
    }

    /// Register the two commands' routes and capabilities on `bridge`.
    fn route_both(bridge: &Bridge) {
        bridge
            .register_routes([
                (SIM_COMMAND.to_string(), Target::SimDriver),
                (HOOK_COMMAND.to_string(), Target::HookDriver),
            ])
            .expect("two new routes merge");
        bridge
            .register_caps([
                (SIM_COMMAND.to_string(), Capability::Command),
                (HOOK_COMMAND.to_string(), Capability::Command),
            ])
            .expect("two new capabilities merge");
    }

    /// The capability set that covers both commands.
    fn commanding() -> HashSet<Capability> {
        [Capability::Command].into_iter().collect()
    }

    /// A routed command from a token without its capability comes back
    /// uncovered and reaches no ring, and so does one on a routed topic
    /// with no capability registered: nothing says the token covers it.
    /// The route is asked first, so an unrouted topic is unrouted whatever
    /// the token holds.
    #[test]
    fn a_command_the_token_does_not_cover_is_handed_back() {
        let bridge = Bridge::new(24);
        bridge.start_inbound(4, 4);
        route_both(&bridge);
        const ROUTED_ONLY: &str = "dcsbridge.builtin.sim.Uncapped";
        bridge
            .register_routes([(ROUTED_ONLY.to_string(), Target::SimDriver)])
            .unwrap();
        let reading: HashSet<Capability> = [Capability::Read].into_iter().collect();

        let sent = command(1, SIM_COMMAND, b"read-only");
        assert_eq!(
            bridge.deliver(&reading, sent.clone()),
            Delivery::Uncovered(sent)
        );
        let uncapped = command(1, ROUTED_ONLY, b"");
        assert_eq!(
            bridge.deliver(&commanding(), uncapped.clone()),
            Delivery::Uncovered(uncapped)
        );
        let stray = command(1, "dcsbridge.sim.Resync", b"");
        assert_eq!(
            bridge.deliver(&HashSet::new(), stray.clone()),
            Delivery::Unrouted(stray)
        );
        assert_eq!(bridge.poll(Target::SimDriver), Ok(None));
        assert_eq!(
            bridge.unrouted_topic(),
            1,
            "an uncovered command was counted as unrouted"
        );
        assert_eq!(bridge.inbound_busy(Target::SimDriver), 0);
    }

    /// A delivered command is polled from the ring its route names and
    /// from no other, in the order it was delivered, with its sender's id
    /// and its bytes intact, and an empty ring answers `None`.
    #[test]
    fn a_routed_command_is_polled_from_its_ring_and_no_other() {
        let bridge = Bridge::new(20);
        bridge.start_inbound(4, 4);
        bridge.start_inbound(1, 1);
        route_both(&bridge);

        assert_eq!(bridge.poll(Target::SimDriver), Ok(None));
        assert_eq!(bridge.poll(Target::HookDriver), Ok(None));

        assert_eq!(
            bridge.deliver(&commanding(), command(7, SIM_COMMAND, b"first")),
            Delivery::Stored
        );
        assert_eq!(
            bridge.deliver(&commanding(), command(8, HOOK_COMMAND, b"kick")),
            Delivery::Stored
        );
        assert_eq!(
            bridge.deliver(&commanding(), command(9, SIM_COMMAND, b"second")),
            Delivery::Stored
        );

        assert_eq!(
            bridge.poll(Target::HookDriver),
            Ok(Some(command(8, HOOK_COMMAND, b"kick"))),
            "the hook ring did not hold the hook command"
        );
        assert_eq!(
            bridge.poll(Target::HookDriver),
            Ok(None),
            "the hook ring held a sim command"
        );
        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(Some(command(7, SIM_COMMAND, b"first")))
        );
        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(Some(command(9, SIM_COMMAND, b"second")))
        );
        assert_eq!(bridge.poll(Target::SimDriver), Ok(None));
        assert_eq!(
            bridge.unrouted_topic(),
            0,
            "a routed command was counted as unrouted"
        );
        assert_eq!(
            bridge.inbound_busy(Target::SimDriver) + bridge.inbound_busy(Target::HookDriver),
            0,
            "a stored command was counted as turned away"
        );
    }

    /// A command on a topic no route map names comes back, is counted, and
    /// reaches neither ring; it is never defaulted to the sim driver. A
    /// topic with a class and a capability but no route is the same case.
    #[test]
    fn an_unrouted_command_is_counted_and_polled_from_neither() {
        let bridge = Bridge::new(21);
        bridge.start_inbound(4, 4);
        route_both(&bridge);
        const EVENT: &str = "dcsbridge.builtin.sim.UnitDestroyed";
        bridge
            .register_classes([(EVENT.to_string(), RecordClass::Durable)])
            .unwrap();
        bridge
            .register_caps([(EVENT.to_string(), Capability::Read)])
            .unwrap();

        let stray = command(3, "dcsbridge.sim.Resync", b"");
        assert_eq!(
            bridge.deliver(&commanding(), stray.clone()),
            Delivery::Unrouted(stray)
        );
        let outbound_only = command(3, EVENT, b"");
        assert_eq!(
            bridge.deliver(&commanding(), outbound_only.clone()),
            Delivery::Unrouted(outbound_only)
        );
        assert_eq!(bridge.unrouted_topic(), 2);
        assert_eq!(bridge.poll(Target::SimDriver), Ok(None));
        assert_eq!(bridge.poll(Target::HookDriver), Ok(None));
    }

    /// A full ring turns the newest command away and keeps what it holds,
    /// counting the refusal against that ring alone, and takes the next
    /// command once one has been polled.
    #[test]
    fn a_full_ring_refuses_the_newest_and_keeps_the_first() {
        let bridge = Bridge::new(22);
        bridge.start_inbound(2, 1);
        route_both(&bridge);

        assert_eq!(
            bridge.deliver(&commanding(), command(1, SIM_COMMAND, b"a")),
            Delivery::Stored
        );
        assert_eq!(
            bridge.deliver(&commanding(), command(1, SIM_COMMAND, b"b")),
            Delivery::Stored
        );
        let third = command(1, SIM_COMMAND, b"c");
        assert_eq!(
            bridge.deliver(&commanding(), third.clone()),
            Delivery::Busy(third)
        );
        assert_eq!(bridge.inbound_busy(Target::SimDriver), 1);
        assert_eq!(bridge.inbound_busy(Target::HookDriver), 0);
        assert_eq!(
            bridge.unrouted_topic(),
            0,
            "a full ring was counted as no route"
        );

        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(Some(command(1, SIM_COMMAND, b"a"))),
            "the oldest command was not kept"
        );
        assert_eq!(
            bridge.deliver(&commanding(), command(1, SIM_COMMAND, b"d")),
            Delivery::Stored
        );
        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(Some(command(1, SIM_COMMAND, b"b")))
        );
        assert_eq!(
            bridge.poll(Target::SimDriver),
            Ok(Some(command(1, SIM_COMMAND, b"d")))
        );
        assert_eq!(bridge.poll(Target::SimDriver), Ok(None));
    }

    /// Before the rings exist a poll is refused as not started, and a
    /// command, which nothing could have sent, is dropped and counted.
    #[test]
    fn a_poll_before_the_rings_exist_is_refused() {
        let bridge = Bridge::new(23);
        route_both(&bridge);
        assert_eq!(bridge.poll(Target::SimDriver), Err(PollError::NotStarted));
        assert_eq!(bridge.poll(Target::HookDriver), Err(PollError::NotStarted));
        let early = command(1, SIM_COMMAND, b"");
        assert_eq!(
            bridge.deliver(&commanding(), early.clone()),
            Delivery::Unrouted(early)
        );
        assert_eq!(bridge.unrouted_topic(), 1);
        assert_eq!(bridge.inbound_busy(Target::SimDriver), 0);
        assert_eq!(
            PollError::NotStarted.to_string(),
            "configure comes first: the rings are allocated by it"
        );
    }

    /// Every refusal counts under its reason, answered or not, and one
    /// answered with nothing counts as suppressed as well.
    #[test]
    fn a_refusal_counts_by_reason_and_a_withheld_one_as_suppressed_too() {
        let bridge = Bridge::new(29);
        for reason in RejectedReason::ALL {
            assert_eq!(bridge.commands_rejected(reason), 0);
        }
        bridge.rejected(RejectedReason::UnknownTopic, true);
        bridge.rejected(RejectedReason::UnknownTopic, false);
        bridge.rejected(RejectedReason::Busy, false);
        assert_eq!(bridge.commands_rejected(RejectedReason::UnknownTopic), 2);
        assert_eq!(bridge.commands_rejected(RejectedReason::NoCapability), 0);
        assert_eq!(bridge.commands_rejected(RejectedReason::RateLimited), 0);
        assert_eq!(bridge.commands_rejected(RejectedReason::Busy), 1);
        assert_eq!(bridge.rejections_suppressed(), 2);
    }

    /// The total is one window shared by every reader thread: what one
    /// connection admitted, another finds counted.
    #[test]
    fn the_inbound_total_is_one_window_across_connections() {
        let bridge = Bridge::new(31);
        let now = Instant::now();
        assert!(bridge.admit_total(now, 2));
        assert!(bridge.admit_total(now, 2));
        assert!(!bridge.admit_total(now, 2), "a third record fit under two");
        assert!(bridge.admit_total(now + std::time::Duration::from_secs(1), 2));
    }
}
