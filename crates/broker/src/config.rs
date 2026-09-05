//! The broker's configuration: every key it owns, with its default, its
//! tier, and the check a value has to pass to be stored under it.
//!
//! The hook driver is the one reader of `Config\DCSBridge.lua`. It parses
//! the file and hands the broker the keys the broker owns, as a flat table,
//! through `shim.configure`. This module is what each value in that table is
//! checked against, and what the whole table is applied by: no Lua, no
//! threads, a table in and a [`Config`] out, or the reason there is none.
//!
//! A call applies as one swap or not at all. Every value is checked for its
//! type and range, then the cross-key invariants are checked against the
//! values that would be in force, and any failure refuses the whole call and
//! leaves the running configuration as it was. A partial apply would let a
//! paired key drift from the key its basis ties it to.
//!
//! Each key has a tier. A **live** key is read at decision time, so a later
//! `configure` swaps it in. A **restart** key sizes an allocation or binds a
//! socket, decided once at the first `configure` and never revisited; a
//! later call reports a changed one as pending a restart and changes nothing.
//! Invariants are checked against effective values, so a pending value never
//! makes a live one pass or fail.
//!
//! A table is the whole configuration, not a delta: a key the table leaves
//! out takes its default. `ReloadConfig` re-reads the whole file, so a key
//! deleted from the file reverts rather than lingering.

use std::fmt;
use std::net::IpAddr;

use crate::state::Token;

/// When a change to a key takes effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier {
    /// Read at decision time; a later `configure` swaps it in.
    Live,
    /// Decided at the first `configure`; a later change waits for a DCS
    /// restart.
    Restart,
}

/// A value as a configuration table carries it.
///
/// Numbers are Lua numbers, so an integer key is checked to be whole and in
/// range. `tokens` is the one nested key, read into its entries before it
/// reaches here.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A Lua boolean.
    Boolean(bool),
    /// A Lua number, which is a double.
    Number(f64),
    /// A Lua string.
    String(String),
    /// The `tokens` list, each entry already read.
    Tokens(Vec<Token>),
}

/// Why a `configure` was refused. The whole call is, whichever key it names.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The value under `key` is not what the key takes.
    Value {
        /// The key whose value was refused.
        key: &'static str,
        /// What the key takes, in words a person fixes the file with.
        expected: String,
    },
    /// The values that would be in force break a rule that ties two keys.
    Invariant(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Value { key, expected } => write!(f, "`{key}` must be {expected}"),
            Error::Invariant(rule) => f.write_str(rule),
        }
    }
}

impl std::error::Error for Error {}

/// What a `configure` produced: the configuration to put in force, and what
/// the table carried that is not in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    /// The configuration to put in force.
    pub config: Config,
    /// Live keys the table named.
    pub live: usize,
    /// Restart-tier keys the table named with a value other than the one in
    /// force. Each is logged and counted, and nothing about it changes.
    pub pending: Vec<Pending>,
    /// Keys the broker does not own, in the table's order. Counted, and
    /// otherwise ignored: a misspelling is an operator's to notice.
    pub unknown: Vec<String>,
}

/// A restart-tier key whose file value differs from its effective one.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    /// The key.
    pub key: &'static str,
    /// The value in force, which stays.
    pub effective: Value,
    /// The value the table carried, which waits for a restart.
    pub file: Value,
}

/// One key's shape: how a value is checked and stored.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Boolean,
    /// A whole number from `min` to `max`.
    Integer {
        min: u64,
        max: u64,
    },
    /// An IP address, as text.
    Address,
    Tokens,
}

/// A whole number from one to the top of a `u32`.
const COUNT: Kind = Kind::Integer {
    min: 1,
    max: u32::MAX as u64,
};

/// A whole number from one up, as wide as a `u64`.
const WIDE: Kind = Kind::Integer {
    min: 1,
    max: u64::MAX,
};

/// A key the broker owns.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    /// The key's name in the file, which is the field's name here.
    pub name: &'static str,
    /// When a change to it takes effect.
    pub tier: Tier,
    kind: Kind,
}

/// Declare the keys once: the field, its type, its default, its tier and its
/// shape. The struct, its default, the key table, and the `get` and `store`
/// over the names all come out of the one list.
macro_rules! keys {
    ($( $field:ident : $ty:ty = $default:expr, $tier:ident, $kind:expr; )*) => {
        /// The broker's effective configuration.
        ///
        /// Every field is a key the specification's defaults table marks as
        /// the broker's, under the key's own name, and that table holds each
        /// one's basis.
        #[derive(Clone, Debug, PartialEq)]
        #[allow(missing_docs)]
        pub struct Config {
            $( pub $field: $ty, )*
        }

        impl Default for Config {
            fn default() -> Self {
                Config { $( $field: $default, )* }
            }
        }

        /// Every key the broker owns, in the specification's order.
        pub static KEYS: &[Key] = &[
            $( Key { name: stringify!($field), tier: Tier::$tier, kind: $kind }, )*
        ];

        impl Config {
            /// The value in force under `name`, or `None` for a key the
            /// broker does not own.
            pub fn get(&self, name: &str) -> Option<Value> {
                match name {
                    $( stringify!($field) => Some(Value::from(&self.$field)), )*
                    _ => None,
                }
            }

            /// Store a checked value under its key.
            fn store(&mut self, key: &Key, value: Value) {
                match key.name {
                    $( stringify!($field) => self.$field = <$ty>::from(value), )*
                    _ => unreachable!("a key from KEYS names a field"),
                }
            }
        }
    };
}

const MIB: u64 = 1 << 20;

keys! {
    // Connections and framing.
    enabled: bool = true, Live, Kind::Boolean;
    bind_address: IpAddr = IpAddr::from([127, 0, 0, 1]), Restart, Kind::Address;
    port: u16 = 7742, Restart, Kind::Integer { min: 0, max: 65535 };
    allow_public_bind: bool = false, Restart, Kind::Boolean;
    max_connections: u32 = 8, Restart, COUNT;
    max_unauthenticated_connections: u32 = 4, Live, COUNT;
    handshake_timeout_ms: u64 = 5000, Live, WIDE;
    max_frame_bytes: u32 = MIB as u32, Live, COUNT;
    max_type_url_bytes: u32 = 256, Live, COUNT;
    rejected_max_per_sec: u32 = 10, Live, COUNT;
    busy_max_per_sec: u32 = 100, Live, COUNT;
    inbound_records_per_sec: u32 = 100, Live, COUNT;
    inbound_records_per_sec_total: u32 = 400, Live, COUNT;
    auth_failures_per_min: u32 = 5, Live, COUNT;
    tokens: Vec<Token> = Vec::new(), Live, Kind::Tokens;
    // Rings. The reserve is a watermark tested on push, so it is live where
    // the sizes are not, and zero is a ring with no reserve.
    ring_out_records: u32 = 4096, Restart, COUNT;
    ring_out_lifecycle_reserve: u32 = 64, Live, Kind::Integer { min: 0, max: u32::MAX as u64 };
    ring_in_sim_driver_records: u32 = 1024, Restart, COUNT;
    ring_in_hook_driver_records: u32 = 256, Restart, COUNT;
    // Timing. The last two are the consumer's, published as advice in the
    // `Schema` reply and read by nothing here.
    heartbeat_interval_ms: u64 = 1000, Live, WIDE;
    dcs_alive_threshold_ms: u64 = 30_000, Live, WIDE;
    dcs_alive_threshold_loading_ms: u64 = 120_000, Live, WIDE;
    load_timeout_ms: u64 = 120_000, Live, WIDE;
    request_timeout_ms: u64 = 2000, Live, WIDE;
    // Spool.
    spool_max_bytes: u64 = 256 * MIB, Live, WIDE;
    spool_retention_hours: u32 = 24, Live, COUNT;
    // Schema and registration. The filter cap bounds a per-connection
    // allocation a consumer asks for, so it is live where the slots are not.
    max_lifecycle_record_bytes: u32 = 16 * 1024, Restart, COUNT;
    max_lifecycle_topics: u32 = 64, Restart, COUNT;
    topic_filter_max_topics: u32 = 256, Live, COUNT;
}

impl From<&bool> for Value {
    fn from(b: &bool) -> Self {
        Value::Boolean(*b)
    }
}

impl From<&IpAddr> for Value {
    fn from(addr: &IpAddr) -> Self {
        Value::String(addr.to_string())
    }
}

impl From<&Vec<Token>> for Value {
    fn from(tokens: &Vec<Token>) -> Self {
        Value::Tokens(tokens.clone())
    }
}

/// A checked integer is a whole number a `u64` holds, and every integer
/// field is at most that wide, so the conversion out of a checked value
/// cannot fail and a number too wide for its field is a defect in `KEYS`.
macro_rules! integer_conversions {
    ($($ty:ty),*) => {$(
        impl From<&$ty> for Value {
            fn from(n: &$ty) -> Self {
                Value::Number(*n as f64)
            }
        }

        impl From<Value> for $ty {
            fn from(value: Value) -> Self {
                match value {
                    Value::Number(n) => <$ty>::try_from(n as u64).expect("checked against the key's range"),
                    _ => unreachable!("checked against the key's kind"),
                }
            }
        }
    )*};
}

integer_conversions!(u16, u32, u64);

impl From<Value> for bool {
    fn from(value: Value) -> Self {
        match value {
            Value::Boolean(b) => b,
            _ => unreachable!("checked against the key's kind"),
        }
    }
}

impl From<Value> for IpAddr {
    fn from(value: Value) -> Self {
        match value {
            Value::String(s) => s.parse().expect("checked against the key's kind"),
            _ => unreachable!("checked against the key's kind"),
        }
    }
}

impl From<Value> for Vec<Token> {
    fn from(value: Value) -> Self {
        match value {
            Value::Tokens(tokens) => tokens,
            _ => unreachable!("checked against the key's kind"),
        }
    }
}

impl Key {
    /// The key named `name`, if the broker owns it.
    pub fn named(name: &str) -> Option<&'static Key> {
        KEYS.iter().find(|key| key.name == name)
    }

    /// Check `value` against this key's shape: a whole number in range, an
    /// address that parses, a boolean or a token list where one is asked.
    fn check(&self, value: &Value) -> Result<(), Error> {
        let refused = |expected: String| Error::Value {
            key: self.name,
            expected,
        };
        match (self.kind, value) {
            (Kind::Boolean, Value::Boolean(_)) => Ok(()),
            (Kind::Boolean, _) => Err(refused("true or false".into())),
            (Kind::Integer { min, max }, Value::Number(n)) => {
                // A double compares exactly against an integer under 2^53,
                // and the caps that are wider are compared as doubles too,
                // so a number at the top of a u64 is not read as one past it.
                let whole = n.fract() == 0.0 && *n >= min as f64 && *n <= max as f64;
                if whole {
                    Ok(())
                } else if max == u64::MAX {
                    Err(refused(format!("a whole number of at least {min}")))
                } else {
                    Err(refused(format!("a whole number from {min} to {max}")))
                }
            }
            (Kind::Integer { .. }, _) => Err(refused("a number".into())),
            (Kind::Address, Value::String(s)) => match s.parse::<IpAddr>() {
                Ok(_) => Ok(()),
                Err(_) => Err(refused("an IP address".into())),
            },
            (Kind::Address, _) => Err(refused("an IP address in a string".into())),
            (Kind::Tokens, Value::Tokens(_)) => Ok(()),
            (Kind::Tokens, _) => Err(refused("a list of token entries".into())),
        }
    }
}

impl Config {
    /// Set `key` to `value`, or say why the value is not one it takes,
    /// leaving the field as it was.
    pub fn set(&mut self, key: &Key, value: Value) -> Result<(), Error> {
        key.check(&value)?;
        self.store(key, value);
        Ok(())
    }

    /// The first `configure`: every key applies, whatever its tier, and a
    /// key the table leaves out takes its default.
    pub fn first<S, I>(table: I) -> Result<Applied, Error>
    where
        S: AsRef<str>,
        I: IntoIterator<Item = (S, Value)>,
    {
        Config::default().merge(table, true)
    }

    /// A later `configure` over the configuration in force: the live keys
    /// apply, a live key the table leaves out takes its default, and a
    /// changed restart-tier key is reported and left as it is.
    pub fn apply<S, I>(&self, table: I) -> Result<Applied, Error>
    where
        S: AsRef<str>,
        I: IntoIterator<Item = (S, Value)>,
    {
        self.merge(table, false)
    }

    fn merge<S, I>(&self, table: I, every_tier: bool) -> Result<Applied, Error>
    where
        S: AsRef<str>,
        I: IntoIterator<Item = (S, Value)>,
    {
        let mut next = Config::default();
        let mut applied = Applied {
            config: Config::default(),
            live: 0,
            pending: Vec::new(),
            unknown: Vec::new(),
        };
        // A restart-tier key keeps its effective value until the table says
        // otherwise, and after the first call the table never does.
        for key in KEYS.iter().filter(|key| key.tier == Tier::Restart) {
            next.store(key, self.get(key.name).expect("a key from KEYS"));
        }

        for (name, value) in table {
            let Some(key) = Key::named(name.as_ref()) else {
                applied.unknown.push(name.as_ref().to_owned());
                continue;
            };
            match key.tier {
                Tier::Live => {
                    applied.live += 1;
                    next.set(key, value)?;
                }
                Tier::Restart if every_tier => next.set(key, value)?,
                Tier::Restart => {
                    // Set on a scratch copy, so a bad value is refused now
                    // rather than at the restart, and so the comparison is
                    // between two values of the field's own type: `1e3`
                    // against `1000`, an address against an address.
                    let mut file = next.clone();
                    file.set(key, value)?;
                    let (effective, file) = (next.get(key.name), file.get(key.name));
                    if effective != file {
                        applied.pending.push(Pending {
                            key: key.name,
                            effective: effective.expect("a key from KEYS"),
                            file: file.expect("a key from KEYS"),
                        });
                    }
                }
            }
        }

        next.check_invariants()?;
        applied.config = next;
        Ok(applied)
    }

    /// The rules that tie one key to another, checked against the values
    /// that would be in force. Each rule's basis is beside its key in the
    /// specification's defaults table.
    fn check_invariants(&self) -> Result<(), Error> {
        if self.max_unauthenticated_connections >= self.max_connections {
            return Err(Error::Invariant(
                "`max_unauthenticated_connections` must be below `max_connections`, so a slowloris cannot exhaust the pool",
            ));
        }
        if self.ring_out_lifecycle_reserve >= self.ring_out_records {
            return Err(Error::Invariant(
                "`ring_out_lifecycle_reserve` must be below `ring_out_records`, or nothing but LIFECYCLE ever fits",
            ));
        }
        if self.heartbeat_interval_ms >= self.dcs_alive_threshold_ms {
            return Err(Error::Invariant(
                "`heartbeat_interval_ms` must be below `dcs_alive_threshold_ms`, or the sim reads dead between beats",
            ));
        }
        if self.dcs_alive_threshold_loading_ms < self.load_timeout_ms {
            return Err(Error::Invariant(
                "`dcs_alive_threshold_loading_ms` must be at least `load_timeout_ms`, or a normal mission load reads as a dead sim",
            ));
        }
        if !self.allow_public_bind && !is_private(self.bind_address) {
            return Err(Error::Invariant(
                "`bind_address` is public; set `allow_public_bind` to listen there anyway",
            ));
        }
        Ok(())
    }
}

/// Whether an address is loopback or private: one the broker binds to
/// without `allow_public_bind`. The unspecified address is public, because
/// it binds every interface.
fn is_private(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Capability;

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    /// Every key has a default and reads back under its own name, the
    /// defaults are the specification's, and a key another component owns
    /// is not here.
    #[test]
    fn every_key_has_a_default() {
        let config = Config::default();
        for key in KEYS {
            assert!(config.get(key.name).is_some(), "{} has no value", key.name);
            assert!(std::ptr::eq(Key::named(key.name).unwrap(), key));
        }
        assert_eq!(KEYS.len(), 29, "a key was added without a row here");

        assert_eq!(config.get("port"), Some(n(7742.0)));
        assert_eq!(
            config.get("bind_address"),
            Some(Value::String("127.0.0.1".into()))
        );
        assert_eq!(config.get("enabled"), Some(Value::Boolean(true)));
        assert_eq!(config.get("tokens"), Some(Value::Tokens(Vec::new())));
        assert_eq!(config.get("route"), None, "a hook driver key");
        assert!(Key::named("max_spots").is_none(), "a sim driver key");
        assert_eq!(Key::named("port").unwrap().tier, Tier::Restart);
        assert_eq!(Key::named("tokens").unwrap().tier, Tier::Live);
    }

    /// A value of each shape sets its field and reads back as itself.
    #[test]
    fn a_checked_value_is_stored() {
        let token = Token {
            id: "map".into(),
            secret: b"s".to_vec(),
            caps: [Capability::Read].into_iter().collect(),
        };
        let mut config = Config::default();
        for (name, value) in [
            ("port", n(0.0)),
            ("bind_address", Value::String("::1".into())),
            ("enabled", Value::Boolean(false)),
            ("handshake_timeout_ms", n(250.0)),
            ("tokens", Value::Tokens(vec![token.clone()])),
        ] {
            let key = Key::named(name).unwrap();
            config.set(key, value.clone()).expect("a valid value");
            assert_eq!(config.get(name), Some(value), "{name} did not round-trip");
        }
        assert_eq!(config.port, 0);
        assert_eq!(config.bind_address, IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]));
        assert!(!config.enabled);
        assert_eq!(config.handshake_timeout_ms, 250);
        assert_eq!(config.tokens, vec![token]);
    }

    /// A value of the wrong type, a fraction, a number out of range and a
    /// string that is not an address are each refused, naming the key and
    /// what it takes, and the field keeps its value.
    #[test]
    fn a_bad_value_is_refused_and_the_field_keeps_its_value() {
        let mut config = Config::default();
        let mut refused = |name: &str, value: Value| {
            let key = Key::named(name).unwrap();
            let before = config.get(name);
            let error = config.set(key, value).expect_err("refused");
            assert_eq!(config.get(name), before, "{name} moved on a refusal");
            match error {
                Error::Value { key, expected } => {
                    assert_eq!(key, name);
                    expected
                }
                Error::Invariant(rule) => panic!("an invariant fired: {rule}"),
            }
        };

        assert_eq!(refused("enabled", n(1.0)), "true or false");
        assert_eq!(
            refused("port", n(65536.0)),
            "a whole number from 0 to 65535"
        );
        assert_eq!(refused("port", n(1.5)), "a whole number from 0 to 65535");
        assert_eq!(refused("port", Value::String("7742".into())), "a number");
        assert_eq!(
            refused("max_connections", n(0.0)),
            "a whole number from 1 to 4294967295"
        );
        assert_eq!(
            refused("handshake_timeout_ms", n(-1.0)),
            "a whole number of at least 1"
        );
        assert_eq!(
            refused("handshake_timeout_ms", n(f64::NAN)),
            "a whole number of at least 1"
        );
        assert_eq!(
            refused("bind_address", Value::String("localhost".into())),
            "an IP address"
        );
        assert_eq!(
            refused("bind_address", n(127.0)),
            "an IP address in a string"
        );
        assert_eq!(
            refused("tokens", Value::String("x".into())),
            "a list of token entries"
        );
        assert_eq!(
            Error::Value {
                key: "port",
                expected: "a number".into()
            }
            .to_string(),
            "`port` must be a number"
        );
    }

    /// A value at the top of a `u64` is accepted as itself: a double
    /// compared as a double neither rounds it past the cap nor refuses it.
    /// A `u32` key refuses the number one past its top.
    #[test]
    fn the_widest_integer_round_trips() {
        let mut config = Config::default();
        let top = u64::MAX as f64;
        config
            .set(Key::named("spool_max_bytes").unwrap(), n(top))
            .expect("in range");
        assert_eq!(config.spool_max_bytes as f64, top);
        assert!(
            config
                .set(Key::named("spool_max_bytes").unwrap(), n(top * 2.0))
                .is_err()
        );
        assert!(
            config
                .set(
                    Key::named("max_frame_bytes").unwrap(),
                    n(u32::MAX as f64 + 1.0)
                )
                .is_err()
        );
        assert!(
            config
                .set(Key::named("ring_out_lifecycle_reserve").unwrap(), n(0.0))
                .is_ok()
        );
    }

    /// A first call over an empty table is the defaults, in force. Over a
    /// full one it applies every tier, a key left out takes its default,
    /// and unknown keys are named rather than refused.
    #[test]
    fn the_first_call_applies_every_tier() {
        let applied = Config::first(Vec::<(&str, Value)>::new()).expect("the defaults hold");
        assert_eq!(applied.config, Config::default());
        assert_eq!((applied.live, applied.pending.len()), (0, 0));
        assert!(applied.unknown.is_empty());

        let token = Token {
            id: "map".into(),
            secret: b"s".to_vec(),
            caps: [Capability::Read].into_iter().collect(),
        };
        let applied = Config::first([
            ("port", n(0.0)),
            ("bind_address", Value::String("::1".into())),
            ("ring_out_records", n(128.0)),
            ("handshake_timeout_ms", n(250.0)),
            ("tokens", Value::Tokens(vec![token.clone()])),
            ("route", Value::String("A".into())),
            ("max_spots", n(16.0)),
        ])
        .expect("a valid table");

        assert_eq!(applied.config.port, 0);
        assert_eq!(
            applied.config.bind_address,
            IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])
        );
        assert_eq!(applied.config.ring_out_records, 128);
        assert_eq!(applied.config.handshake_timeout_ms, 250);
        assert_eq!(applied.config.tokens, vec![token]);
        assert_eq!(applied.config.max_connections, 8, "a key left out");
        assert_eq!(applied.live, 2);
        assert!(applied.pending.is_empty());
        assert_eq!(applied.unknown, ["route", "max_spots"]);
    }

    /// A later call swaps the live keys, reverts a live key the table
    /// dropped, and reports a changed restart-tier key without applying it.
    /// An unchanged one, however the file spells it, is not pending.
    #[test]
    fn a_later_call_applies_live_keys_and_reports_restart_keys() {
        let first = Config::first([
            ("port", n(0.0)),
            ("bind_address", Value::String("::1".into())),
            ("handshake_timeout_ms", n(250.0)),
        ])
        .expect("a valid table")
        .config;

        let later = first
            .apply([
                ("rejected_max_per_sec", n(20.0)),
                ("port", n(7743.0)),
                ("ring_out_records", n(4096.0)),
                ("bind_address", Value::String("0:0:0:0:0:0:0:1".into())),
                ("max_connections", n(16.0)),
            ])
            .expect("a valid table");

        assert_eq!(later.config.rejected_max_per_sec, 20);
        assert_eq!(later.config.handshake_timeout_ms, 5000, "a dropped key");
        assert_eq!(later.config.port, 0, "a restart-tier key moved");
        assert_eq!(later.config.max_connections, 8);
        assert_eq!(later.live, 1);
        assert_eq!(
            later.pending,
            [
                Pending {
                    key: "port",
                    effective: n(0.0),
                    file: n(7743.0)
                },
                Pending {
                    key: "max_connections",
                    effective: n(8.0),
                    file: n(16.0)
                },
            ]
        );
    }

    /// A bad value anywhere in the table refuses the whole call: a good
    /// key before it does not apply, and a restart-tier key carrying one is
    /// refused on a later call rather than at the restart.
    #[test]
    fn a_bad_value_refuses_the_whole_call() {
        let config = Config::default();
        let refused = config
            .apply([("enabled", Value::Boolean(false)), ("port", n(-1.0))])
            .expect_err("refused");
        assert_eq!(
            refused.to_string(),
            "`port` must be a whole number from 0 to 65535"
        );
        assert!(config.apply([("ring_out_records", n(0.5))]).is_err());
        assert!(Config::first([("tokens", n(1.0))]).is_err());
    }

    /// Each rule that ties two keys refuses the call, and it is checked
    /// against the values that would be in force: a pending restart-tier
    /// value neither passes a live key nor fails it.
    #[test]
    fn invariants_are_checked_against_effective_values() {
        let invariant = |table: Vec<(&str, Value)>| match Config::first(table) {
            Err(Error::Invariant(rule)) => rule,
            other => panic!("expected an invariant to fire, got {other:?}"),
        };

        assert!(
            invariant(vec![("max_unauthenticated_connections", n(8.0))])
                .contains("`max_connections`")
        );
        assert!(
            invariant(vec![("ring_out_lifecycle_reserve", n(4096.0))])
                .contains("`ring_out_records`")
        );
        assert!(
            invariant(vec![("heartbeat_interval_ms", n(30_000.0))])
                .contains("`dcs_alive_threshold_ms`")
        );
        assert!(invariant(vec![("load_timeout_ms", n(120_001.0))]).contains("`load_timeout_ms`"));
        assert!(
            invariant(vec![("bind_address", Value::String("0.0.0.0".into()))])
                .contains("`allow_public_bind`")
        );

        // Raising both sides together passes, and so does a public bind
        // with the gate set. A private or loopback address needs no gate.
        Config::first([
            ("max_connections", n(16.0)),
            ("max_unauthenticated_connections", n(8.0)),
        ])
        .expect("both raised");
        Config::first([
            ("bind_address", Value::String("0.0.0.0".into())),
            ("allow_public_bind", Value::Boolean(true)),
        ])
        .expect("gated");
        for private in [
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            Config::first([("bind_address", Value::String(private.into()))])
                .unwrap_or_else(|e| panic!("{private} needs no gate: {e}"));
        }

        // In force: 16 connections, 8 unauthenticated. A file that lowers
        // `max_connections` to 8 has a pending value; the effective 16 is
        // what the invariant sees, so the call passes.
        let config = Config::first([
            ("max_connections", n(16.0)),
            ("max_unauthenticated_connections", n(8.0)),
        ])
        .expect("valid")
        .config;
        let later = config
            .apply([
                ("max_connections", n(8.0)),
                ("max_unauthenticated_connections", n(8.0)),
            ])
            .expect("the effective cap is 16");
        assert_eq!(later.pending.len(), 1);
        assert_eq!(later.config.max_unauthenticated_connections, 8);

        // And a live key that breaks the rule against the effective value
        // is refused, however the file's pending value would read.
        assert!(matches!(
            config.apply([
                ("max_connections", n(64.0)),
                ("max_unauthenticated_connections", n(16.0)),
            ]),
            Err(Error::Invariant(_))
        ));
    }
}
