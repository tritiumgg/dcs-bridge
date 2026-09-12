//! `luaopen_dcsbridge`, and the Lua declarations behind it.
//!
//! DCS loads the broker by explicit path rather than through `require`,
//! because `package.cpath` is not set in the hook state:
//!
//! ```lua
//! local path   = lfs.writedir() .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'
//! local loader = assert(package.loadlib(path, 'luaopen_dcsbridge'))
//! local shim   = loader()
//! ```
//!
//! The same call opens the host-native `.so` or `.dylib` a stock Lua 5.1
//! builds against, which is what lets the broker's behaviour be tested with no
//! DCS present.
//!
//! This crate holds the whole Lua surface and no broker logic. The rings,
//! threads and framing live in `dcsbridge-broker`, which names no Lua symbol
//! and so links into a test binary on any host. ADR 0005.

/// DCS's Lua, or the host's — `build.rs` and the `dcs-lua` feature decide
/// which. ADR 0006.
///
/// The public API is stock Lua 5.1, so these are the stock declarations, and
/// `vendor/lua/lua.def` decides which of the 114 exports the broker may name.
/// `lua_newtable`, `lua_pushcfunction`, `lua_pop` and `lua_upvalueindex` are
/// macros over the functions named here and are not exported symbols.
///
/// On Windows the symbols come from `lua.dll` through an import library, so
/// they exist only where that library does. Everywhere else they are left
/// undefined and resolved from the interpreter that opens the module.
#[cfg(any(unix, feature = "dcs-lua"))]
mod lua {
    use core::ffi::{c_char, c_int, c_void};

    /// A function Lua can call: it takes its arguments off the stack and
    /// returns how many results it left there.
    pub type CFunction = unsafe extern "C" fn(state: *mut c_void) -> c_int;

    /// The pseudo-index of a closure's first upvalue: `lua_upvalueindex(1)`,
    /// which 5.1 defines as `LUA_GLOBALSINDEX - 1` with `LUA_GLOBALSINDEX`
    /// at -10002.
    pub const UPVALUE_1: c_int = -10003;

    /// The type tag of nil, for `lua_type`.
    pub const TNIL: c_int = 0;

    /// The type tag of a boolean, for `luaL_checktype`.
    pub const TBOOLEAN: c_int = 1;

    /// The type tag of a number, for `luaL_checktype`.
    pub const TNUMBER: c_int = 3;

    /// The type tag of a string, for `lua_type`.
    pub const TSTRING: c_int = 4;

    /// The type tag of a table, for `luaL_checktype` and `lua_type`.
    pub const TTABLE: c_int = 5;

    unsafe extern "C" {
        /// Push a fresh table sized for `narr` array and `nrec` hash entries.
        pub unsafe fn lua_createtable(state: *mut c_void, narr: c_int, nrec: c_int);

        /// Push `n`. `lua_Integer` is `ptrdiff_t` in a stock 5.1 build.
        pub unsafe fn lua_pushinteger(state: *mut c_void, n: isize);

        /// Push the `len` bytes at `s` as a string, which Lua copies.
        pub unsafe fn lua_pushlstring(state: *mut c_void, s: *const c_char, len: usize);

        /// Push a boolean; any non-zero `b` is true.
        pub unsafe fn lua_pushboolean(state: *mut c_void, b: c_int);

        /// Push a copy of the value at `index`.
        pub unsafe fn lua_pushvalue(state: *mut c_void, index: c_int);

        /// Pop `n` values and push `f` closed over them.
        pub unsafe fn lua_pushcclosure(state: *mut c_void, f: CFunction, n: c_int);

        /// Push a fresh block of `size` bytes that Lua owns and collects.
        pub unsafe fn lua_newuserdata(state: *mut c_void, size: usize) -> *mut c_void;

        /// The block behind the userdata at `index`, or null.
        pub unsafe fn lua_touserdata(state: *mut c_void, index: c_int) -> *mut c_void;

        /// Pop the top value into `key` of the table at `index`.
        pub unsafe fn lua_setfield(state: *mut c_void, index: c_int, key: *const c_char);

        /// Pop the top value and make it the metatable of the value at `index`.
        pub unsafe fn lua_setmetatable(state: *mut c_void, index: c_int) -> c_int;

        /// Set the stack height; a negative `index` counts from the top.
        pub unsafe fn lua_settop(state: *mut c_void, index: c_int);

        /// The number at `narg`, or raise an argument error.
        pub unsafe fn luaL_checknumber(state: *mut c_void, narg: c_int) -> f64;

        /// The number at `index`, or zero for anything that is not one.
        /// Unlike `luaL_checknumber` it converts a string, so a type check
        /// comes first where a string must not pass.
        pub unsafe fn lua_tonumber(state: *mut c_void, index: c_int) -> f64;

        /// Raise an error blaming argument `narg` with `extramsg`. Never
        /// returns.
        pub unsafe fn luaL_argerror(
            state: *mut c_void,
            narg: c_int,
            extramsg: *const c_char,
        ) -> c_int;

        /// The string at `narg` with its length in `len`, or raise.
        pub unsafe fn luaL_checklstring(
            state: *mut c_void,
            narg: c_int,
            len: *mut usize,
        ) -> *const c_char;

        /// Raise unless the value at `narg` has type `t`.
        pub unsafe fn luaL_checktype(state: *mut c_void, narg: c_int, t: c_int);

        /// The type tag of the value at `index`.
        pub unsafe fn lua_type(state: *mut c_void, index: c_int) -> c_int;

        /// Push `t[key]` for the table at `index`.
        pub unsafe fn lua_getfield(state: *mut c_void, index: c_int, key: *const c_char);

        /// Push `t[n]` for the table at `index`, without metamethods.
        pub unsafe fn lua_rawgeti(state: *mut c_void, index: c_int, n: c_int);

        /// The stack's height, which is also the absolute index of its top.
        pub unsafe fn lua_gettop(state: *mut c_void) -> c_int;

        /// Push nil.
        pub unsafe fn lua_pushnil(state: *mut c_void);

        /// Pop a key and push the next key and value of the table at
        /// `index`, or pop the key and push nothing at the end. Returns
        /// whether a pair was pushed.
        pub unsafe fn lua_next(state: *mut c_void, index: c_int) -> c_int;

        /// The string at `index` with its length in `len`, or null for a
        /// value that is not a string or a number.
        pub unsafe fn lua_tolstring(
            state: *mut c_void,
            index: c_int,
            len: *mut usize,
        ) -> *const c_char;

        /// The truth of the value at `index`.
        pub unsafe fn lua_toboolean(state: *mut c_void, index: c_int) -> c_int;

        /// Raise an error formatted like `printf`. Never returns.
        pub unsafe fn luaL_error(state: *mut c_void, fmt: *const c_char, ...) -> c_int;

        /// Raise the value at the top of the stack as an error. Never
        /// returns. Unlike `luaL_error` it formats nothing, so a message
        /// built on the Rust side is pushed and dropped by `push_error`,
        /// and then raised from a frame that owns nothing.
        pub unsafe fn lua_error(state: *mut c_void) -> c_int;

        /// Push `n`.
        pub unsafe fn lua_pushnumber(state: *mut c_void, n: f64);

        /// Pop the top value into `t[n]` for the table at `index`, without
        /// metamethods.
        pub unsafe fn lua_rawseti(state: *mut c_void, index: c_int, n: c_int);
    }
}

/// The Interface A call surface: the calls on the table `luaopen_dcsbridge`
/// leaves, their arguments and what they return.
///
/// The hook driver compares it at its first `configure` and disables itself
/// on a mismatch, so it moves when a call is added, removed or changes
/// signature, and for nothing else. It is an opaque equality, not an order.
pub const INTERFACE_VERSION: &str = "5";

/// Open the bridge in `state`, leaving one table on the stack.
///
/// The table is this state's own and the bridge behind it is the process's.
/// Both DCS states load this module, so this runs more than once and each call
/// gets its own table over one set of rings, sockets and registration maps.
/// ADR 0007.
///
/// The table carries the broker version, the interface version, `opens`, the
/// number of times the module has been opened in this process, `configure`,
/// `schema`, `tick`, `epoch`, `poll`, the registration calls and the put
/// calls. The first table reads 1 and the second reads 2, which is how two
/// tables are shown to sit over one bridge.
///
/// An open allocates nothing and listens on nothing. The first
/// `shim.configure` does both, from the configuration it is handed, so a
/// state that opens the module after that call finds the bridge running and
/// disturbs it in no way.
///
/// # Safety
///
/// `state` must be a live `lua_State` from the Lua that opened this module,
/// with room for four stack slots. It is called by Lua, which guarantees both.
#[cfg(any(unix, feature = "dcs-lua"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaopen_dcsbridge(state: *mut core::ffi::c_void) -> core::ffi::c_int {
    let version = dcsbridge_broker::BROKER_VERSION;
    let opens = dcsbridge_broker::bridge().open();

    // SAFETY: the caller guarantees a live lua_State with four free slots. The
    // table takes one, each pushed value the other, and lua_setfield pops the
    // value back off. Lua copies the bytes it is given, so nothing this crate
    // allocated is left for DCS's C runtime to free.
    unsafe {
        lua::lua_createtable(
            state,
            0,
            8 + (put::CALLS.len() + register::CALLS.len()) as core::ffi::c_int,
        );
        lua::lua_pushlstring(
            state,
            version.as_ptr().cast::<core::ffi::c_char>(),
            version.len(),
        );
        // -1 is the version string just pushed, so the table is at -2.
        lua::lua_setfield(state, -2, c"version".as_ptr());
        lua::lua_pushlstring(
            state,
            INTERFACE_VERSION.as_ptr().cast::<core::ffi::c_char>(),
            INTERFACE_VERSION.len(),
        );
        lua::lua_setfield(state, -2, c"interface".as_ptr());

        // A count that outgrew isize would need more Lua states than DCS has.
        lua::lua_pushinteger(state, opens as isize);
        lua::lua_setfield(state, -2, c"opens".as_ptr());

        put::install(state);
        configure::install(state);
        schema::install(state);
        tick::install(state);
        epoch::install(state);
        register::install(state);
        poll::install(state);
    }

    1
}

/// `shim.configure(table)`: the broker's keys from `Config\DCSBridge.lua`,
/// applied as one swap or refused whole.
///
/// The hook driver is the file's one reader. It hands the broker the keys
/// the broker owns as a flat table of strings, numbers and booleans, plus
/// the `tokens` list, and gets back the interface version it compares
/// against its own, and what the call did with the table. Until the first
/// call allocates and binds, the module open does, at the defaults.
#[cfg(any(unix, feature = "dcs-lua"))]
mod configure {
    use core::ffi::{CStr, c_int, c_void};
    use std::collections::HashSet;

    use dcsbridge_broker::config::{Applied, Value};
    use dcsbridge_broker::registry::Capability;
    use dcsbridge_broker::state::Token;

    use crate::lua;

    /// Put `configure` on the table at the top of the stack.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and one stack slot is free.
    pub unsafe fn install(state: *mut c_void) {
        // SAFETY: the push and the setfield pair, leaving the table on top.
        unsafe {
            lua::lua_pushcclosure(state, configure, 0);
            lua::lua_setfield(state, -2, c"configure".as_ptr());
        }
    }

    /// What a bad token entry is refused with: which entry, and what about
    /// it.
    struct Refused {
        entry: usize,
        why: &'static CStr,
    }

    /// `shim.configure({ port = 7742, tokens = { { id = 'map', secret = '...',
    /// caps = { 'read' } } }, ... })`.
    ///
    /// Every key is a string and every value a number, a string or a
    /// boolean, except `tokens`, which is a list of entries each with an
    /// `id` string, a `secret` string and a `caps` list of capability names
    /// or numbers. The whole table is read before any of it takes effect,
    /// so a bad value leaves the configuration in force as it was, and the
    /// error names the key. A table that reads applies as the broker
    /// applies it: every key at the first call, the live keys after.
    ///
    /// Returns a table: `interface`, the version the hook driver compares;
    /// `applied`, the live keys the table named; `unknown`, the keys the
    /// broker does not own; and `pending`, a list of `{ key, effective,
    /// file }` for each restart-tier key whose file value differs from the
    /// one in force, with `pending_restart` its length.
    unsafe extern "C" fn configure(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call. This frame owns nothing, which is what lets it
        // raise; see `push_error`.
        unsafe {
            if apply(state) {
                return 1;
            }
            lua::lua_error(state);
        }
        unreachable!("lua_error does not return")
    }

    /// Read the table, apply it, and push the answer, or push the refusal.
    /// Returns whether the answer was pushed.
    ///
    /// # Safety
    ///
    /// `state` is inside a Lua call whose first argument is at 1, with
    /// five stack slots free.
    // Never inlined into the entry: the entry raises, and a raise crosses
    // this frame only if this frame is not there. See `push_error`.
    #[inline(never)]
    unsafe fn apply(state: *mut c_void) -> bool {
        // SAFETY: the argument is a table for the whole call, and every
        // value pushed while reading it is popped before the read returns.
        unsafe {
            lua::luaL_checktype(state, 1, lua::TTABLE);
            let table = match read_table(state) {
                Ok(table) => table,
                Err(why) => return refuse(state, why),
            };
            match dcsbridge_broker::bridge().configure(table) {
                Ok(applied) => {
                    push_applied(state, &applied);
                    true
                }
                Err(error) => refuse(state, error.to_string()),
            }
        }
    }

    /// Push `configure refused: <why>` for the caller to raise. Always
    /// false.
    unsafe fn refuse(state: *mut c_void, why: String) -> bool {
        // SAFETY: the caller's contract.
        unsafe { crate::push_error(state, format!("configure refused: {why}")) }
        false
    }

    /// Read the flat table at argument 1 into its keys and values, or say
    /// which key is not one the call takes. Leaves the stack as found.
    ///
    /// # Safety
    ///
    /// `state` is live, argument 1 is a table, and five stack slots are
    /// free.
    unsafe fn read_table(state: *mut c_void) -> Result<Vec<(String, Value)>, String> {
        // SAFETY: each `lua_next` pops the key it was given and pushes a
        // pair; the value is popped before the next call and both are
        // popped on the way out of a refusal. The key is checked to be a
        // string before it is read as one, because reading a number key as
        // a string converts it in place and breaks the walk.
        unsafe {
            let mut table = Vec::new();
            lua::lua_pushnil(state);
            while lua::lua_next(state, 1) != 0 {
                if lua::lua_type(state, -2) != lua::TSTRING {
                    lua::lua_settop(state, -3);
                    return Err("a key is not a string".into());
                }
                let mut len = 0;
                let s = lua::lua_tolstring(state, -2, &mut len);
                let key = String::from_utf8_lossy(core::slice::from_raw_parts(s.cast(), len))
                    .into_owned();
                let value = match lua::lua_type(state, -1) {
                    lua::TBOOLEAN => Ok(Value::Boolean(lua::lua_toboolean(state, -1) != 0)),
                    lua::TNUMBER => Ok(Value::Number(lua::lua_tonumber(state, -1))),
                    lua::TSTRING => {
                        let s = lua::lua_tolstring(state, -1, &mut len);
                        let bytes = core::slice::from_raw_parts(s.cast(), len);
                        Ok(Value::String(String::from_utf8_lossy(bytes).into_owned()))
                    }
                    lua::TTABLE if key == "tokens" => read_list(state, -1)
                        .map(Value::Tokens)
                        .map_err(|Refused { entry, why }| {
                            format!("`tokens` entry {entry} {}", why.to_string_lossy())
                        }),
                    lua::TTABLE => Err(format!("`{key}` is a table, and only `tokens` is one")),
                    _ => Err(format!("`{key}` is not a number, a string or a boolean")),
                };
                lua::lua_settop(state, -2);
                match value {
                    Ok(value) => table.push((key, value)),
                    Err(why) => {
                        lua::lua_settop(state, -2);
                        return Err(why);
                    }
                }
            }
            Ok(table)
        }
    }

    /// Push the table `configure` returns for `applied`.
    ///
    /// # Safety
    ///
    /// `state` is live and five stack slots are free.
    unsafe fn push_applied(state: *mut c_void, applied: &Applied) {
        let interface = crate::INTERFACE_VERSION;
        // SAFETY: every push pairs with the setfield or rawseti that pops
        // it, and the result table stays on top between them.
        unsafe {
            lua::lua_createtable(state, 0, 6);
            lua::lua_pushlstring(state, interface.as_ptr().cast(), interface.len());
            lua::lua_setfield(state, -2, c"interface".as_ptr());
            lua::lua_pushinteger(state, applied.live as isize);
            lua::lua_setfield(state, -2, c"applied".as_ptr());
            lua::lua_pushinteger(state, applied.unknown.len() as isize);
            lua::lua_setfield(state, -2, c"unknown".as_ptr());
            lua::lua_pushinteger(state, applied.pending.len() as isize);
            lua::lua_setfield(state, -2, c"pending_restart".as_ptr());
            if let Some(outbound) = dcsbridge_broker::bridge().outbound() {
                let listening = outbound.local_addr().to_string();
                lua::lua_pushlstring(state, listening.as_ptr().cast(), listening.len());
                lua::lua_setfield(state, -2, c"listening".as_ptr());
            }

            lua::lua_createtable(state, applied.pending.len() as c_int, 0);
            for (n, pending) in applied.pending.iter().enumerate() {
                lua::lua_createtable(state, 0, 3);
                lua::lua_pushlstring(state, pending.key.as_ptr().cast(), pending.key.len());
                lua::lua_setfield(state, -2, c"key".as_ptr());
                push_value(state, &pending.effective);
                lua::lua_setfield(state, -2, c"effective".as_ptr());
                push_value(state, &pending.file);
                lua::lua_setfield(state, -2, c"file".as_ptr());
                lua::lua_rawseti(state, -2, n as c_int + 1);
            }
            lua::lua_setfield(state, -2, c"pending".as_ptr());
        }
    }

    /// Push `value` as the Lua value it came from. A token list is never
    /// pending, since the key is live, so it pushes nil.
    ///
    /// # Safety
    ///
    /// `state` is live and one stack slot is free.
    unsafe fn push_value(state: *mut c_void, value: &Value) {
        // SAFETY: one push, and Lua copies the string's bytes.
        unsafe {
            match value {
                Value::Boolean(b) => lua::lua_pushboolean(state, c_int::from(*b)),
                Value::Number(n) => lua::lua_pushnumber(state, *n),
                Value::String(s) => lua::lua_pushlstring(state, s.as_ptr().cast(), s.len()),
                Value::Tokens(_) => lua::lua_pushnil(state),
            }
        }
    }

    /// Read every entry of the token list at `index`. Leaves the stack as
    /// found.
    ///
    /// # Safety
    ///
    /// `state` is live, a table is at `index`, and four stack slots are
    /// free.
    unsafe fn read_list(state: *mut c_void, index: c_int) -> Result<Vec<Token>, Refused> {
        // SAFETY: the index is made absolute before anything is pushed, and
        // each rawgeti pushes one value the settop below pops, whichever
        // way the entry's read ends.
        unsafe {
            let table = if index < 0 {
                lua::lua_gettop(state) + index + 1
            } else {
                index
            };
            let count = dense_length(state, table).map_err(|entry| Refused {
                entry,
                why: c"is missing: the list has a hole, or a key that is not a position",
            })?;
            let mut list = Vec::with_capacity(count);
            for n in 1..=count {
                lua::lua_rawgeti(state, table, n as c_int);
                let entry = read_entry(state, n);
                lua::lua_settop(state, -2);
                list.push(entry?);
            }
            Ok(list)
        }
    }

    /// The length of the list at `index`, once it is known to be one: every
    /// key a position from 1 to the length with none missing.
    ///
    /// The length operator answers any border of a table with a hole, so a
    /// list with an entry commented out of its middle could read short and
    /// the entries past the hole would be dropped without a word. Walking
    /// every key instead makes a hole, or a key that is not a position, a
    /// refusal naming the first position that is missing. Leaves the stack
    /// as found.
    ///
    /// # Safety
    ///
    /// `state` is live, a table is at `index`, and three stack slots are
    /// free.
    unsafe fn dense_length(state: *mut c_void, index: c_int) -> Result<usize, usize> {
        // SAFETY: the table's index is made absolute before anything is
        // pushed, so the pushes of the walk do not move it. Each `lua_next`
        // pops the key it was given and pushes a pair, and the value is
        // popped before the next call, so the walk ends with the stack as
        // it began; a walk cut short pops both.
        unsafe {
            let table = if index < 0 {
                lua::lua_gettop(state) + index + 1
            } else {
                index
            };
            let mut count = 0usize;
            let mut highest = 0usize;
            lua::lua_pushnil(state);
            while lua::lua_next(state, table) != 0 {
                let key = if lua::lua_type(state, -2) == lua::TNUMBER {
                    lua::lua_tonumber(state, -2)
                } else {
                    f64::NAN
                };
                if !(1.0..=f64::from(c_int::MAX)).contains(&key) || key.fract() != 0.0 {
                    lua::lua_settop(state, -3);
                    return Err(count + 1);
                }
                count += 1;
                highest = highest.max(key as usize);
                lua::lua_settop(state, -2);
            }
            if count == highest {
                return Ok(count);
            }
            for n in 1..=highest {
                lua::lua_rawgeti(state, table, n as c_int);
                let missing = lua::lua_type(state, -1) == lua::TNIL;
                lua::lua_settop(state, -2);
                if missing {
                    return Err(n);
                }
            }
            Ok(count)
        }
    }

    /// Read the entry at the top of the stack, leaving the stack as found.
    ///
    /// # Safety
    ///
    /// `state` is live and three stack slots are free.
    unsafe fn read_entry(state: *mut c_void, entry: usize) -> Result<Token, Refused> {
        let refused = |why: &'static CStr| Refused { entry, why };
        // SAFETY: the entry is at -1 on entry and every push here is popped
        // before the next field is read, so the indices below hold.
        unsafe {
            if lua::lua_type(state, -1) != lua::TTABLE {
                return Err(refused(c"is not a table"));
            }
            let id = field_string(state, c"id").ok_or_else(|| refused(c"has no id string"))?;
            let secret =
                field_string(state, c"secret").ok_or_else(|| refused(c"has no secret string"))?;
            if secret.is_empty() {
                return Err(refused(c"has an empty secret"));
            }

            lua::lua_getfield(state, -1, c"caps".as_ptr());
            let caps = read_caps(state);
            lua::lua_settop(state, -2);
            let caps =
                caps.ok_or_else(|| refused(c"has no caps list of read, command or reload"))?;

            Ok(Token {
                id: String::from_utf8_lossy(&id).into_owned(),
                secret,
                caps,
            })
        }
    }

    /// The string field `key` of the table at -1, or `None` for anything
    /// else. Leaves the stack as found.
    ///
    /// # Safety
    ///
    /// `state` is live, a table is at -1, and one stack slot is free.
    unsafe fn field_string(state: *mut c_void, key: &CStr) -> Option<Vec<u8>> {
        // SAFETY: the getfield pushes one value and the settop pops it; the
        // bytes are copied out before the pop, because Lua owns them.
        unsafe {
            lua::lua_getfield(state, -1, key.as_ptr());
            let value = if lua::lua_type(state, -1) == lua::TSTRING {
                let mut len = 0;
                let s = lua::lua_tolstring(state, -1, &mut len);
                Some(core::slice::from_raw_parts(s.cast::<u8>(), len).to_vec())
            } else {
                None
            };
            lua::lua_settop(state, -2);
            value
        }
    }

    /// The capability list at -1: each element a name or the schema's
    /// number for it. `None` for anything else, or an unknown member.
    /// Leaves the stack as found.
    ///
    /// # Safety
    ///
    /// `state` is live, the list is at -1, and one stack slot is free.
    unsafe fn read_caps(state: *mut c_void) -> Option<HashSet<Capability>> {
        // SAFETY: each rawgeti pushes one value and is popped before the
        // next, and the bytes read are Lua's for that span.
        unsafe {
            if lua::lua_type(state, -1) != lua::TTABLE {
                return None;
            }
            let count = dense_length(state, -1).ok()?;
            let mut caps = HashSet::new();
            for n in 1..=count {
                lua::lua_rawgeti(state, -1, n as c_int);
                let cap = crate::member(state, -1);
                lua::lua_settop(state, -2);
                caps.insert(cap?);
            }
            Some(caps)
        }
    }
}

/// Push `message` as the value a Lua error will carry, and drop it. The
/// caller raises.
///
/// A raise is a `longjmp`, and on x64 Windows a `longjmp` is an unwind:
/// it visits every frame it crosses and consults the exception tables of
/// any frame that has them. What DCS's process does when the frame it
/// jumps out of is a Rust frame full of vectors, strings and destructor
/// funclets is exit, silently, with no report; the tables at the raise
/// name nothing to destroy in that frame, so the exact step is not
/// pinned down. What is pinned down is the shape that survives: a frame
/// that owns nothing at all. Every `extern "C"` entry here that raises is
/// shaped that way: the work, the message and every drop happen in a
/// callee that returns, and the entry then calls `lua_error` holding
/// nothing.
///
/// The callee is `#[inline(never)]`, because the shape is the source's and
/// the optimizer is free to undo it. Inlined into the entry, the callee's
/// vectors and strings become the entry's, and that is what took DCS down
/// at the first refusal raised live from a registration call, while
/// `configure`, whose callee happened to stay a call, survived. The
/// release IR is the check: every raising entry is a few lines, calls its
/// callee, and frees nothing.
///
/// # Safety
///
/// `state` is live and one stack slot is free.
#[cfg(any(unix, feature = "dcs-lua"))]
unsafe fn push_error(state: *mut core::ffi::c_void, message: String) {
    // SAFETY: Lua copies the bytes it is given.
    unsafe { lua::lua_pushlstring(state, message.as_ptr().cast(), message.len()) }
    drop(message);
}

/// The member of a mirrored enum that the value at `index` names, by its
/// lowercase name or the schema's number, or nothing for any other value.
///
/// # Safety
///
/// `state` is live and `index` is on its stack. Nothing is pushed.
#[cfg(any(unix, feature = "dcs-lua"))]
unsafe fn member<V: dcsbridge_broker::registry::Member>(
    state: *mut core::ffi::c_void,
    index: core::ffi::c_int,
) -> Option<V> {
    // SAFETY: the caller's contract; the bytes read are Lua's for the span
    // they are read in, and a number is copied out.
    unsafe {
        match lua::lua_type(state, index) {
            lua::TSTRING => {
                let mut len = 0;
                let s = lua::lua_tolstring(state, index, &mut len);
                V::from_name(core::slice::from_raw_parts(s.cast::<u8>(), len))
            }
            lua::TNUMBER => {
                let n = lua::lua_tonumber(state, index);
                // A fraction or a number past the enum's range is no member,
                // and the cast would round the first and saturate the second
                // onto one.
                if n.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&n) {
                    return None;
                }
                V::from_number(n as u32)
            }
            _ => None,
        }
    }
}

/// `shim.schema(bytes)`: the compiled `FileDescriptorSet` the hook driver
/// read from `Mods\services\DCSBridge\schema.pb`, handed to the broker once.
///
/// The broker holds the bytes, hashes them, serves them from `GetSchema` and
/// puts the hash in every handshake from then on; it parses none of them.
/// The call answers the hash as lowercase hex, which is what a person
/// compares against the file. It comes after the first `configure` and
/// happens once: a second call is refused, because replacing the served set
/// is a DCS restart.
#[cfg(any(unix, feature = "dcs-lua"))]
mod schema {
    use core::ffi::{c_int, c_void};

    use crate::lua;

    /// Put `schema` on the table at the top of the stack.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and one stack slot is free.
    pub unsafe fn install(state: *mut c_void) {
        // SAFETY: the push and the setfield pair, leaving the table on top.
        unsafe {
            lua::lua_pushcclosure(state, schema, 0);
            lua::lua_setfield(state, -2, c"schema".as_ptr());
        }
    }

    /// `shim.schema(bytes)`: hand the bytes over, and answer their SHA-256
    /// in hex. Raises `schema refused: ...` before the first `configure`,
    /// on empty bytes, and once a schema is held.
    unsafe extern "C" fn schema(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call. This frame owns nothing, which is what lets it
        // raise; see `push_error`.
        unsafe {
            if hand_over(state) {
                return 1;
            }
            lua::lua_error(state);
        }
        unreachable!("lua_error does not return")
    }

    /// Hand the bytes over and push the hash, or push the refusal. Returns
    /// whether the hash was pushed.
    ///
    /// # Safety
    ///
    /// `state` is inside a Lua call whose first argument is at 1, with one
    /// stack slot free.
    // Never inlined into the entry; see `apply` in `configure`.
    #[inline(never)]
    unsafe fn hand_over(state: *mut c_void) -> bool {
        // SAFETY: the argument is checked to be a string, so the pointer
        // and length name Lua's own bytes for the whole call, and the
        // broker copies them before anything is pushed.
        unsafe {
            // checklstring alone would take a number as its decimal
            // string, and a number is a call that meant something else.
            lua::luaL_checktype(state, 1, lua::TSTRING);
            let mut len = 0usize;
            let ptr = lua::luaL_checklstring(state, 1, &mut len);
            let bytes = core::slice::from_raw_parts(ptr.cast::<u8>(), len);
            match dcsbridge_broker::bridge().hold_schema(bytes) {
                Ok(sha256) => {
                    let hex: String = sha256.iter().map(|byte| format!("{byte:02x}")).collect();
                    lua::lua_pushlstring(state, hex.as_ptr().cast(), hex.len());
                    drop(hex);
                    true
                }
                Err(error) => {
                    crate::push_error(state, format!("schema refused: {error}"));
                    false
                }
            }
        }
    }
}

/// `shim.tick(mission_time)`: the sim's clock, from the hook driver's
/// per-frame callback.
///
/// The broker publishes the mission time on every call and stamps the
/// heartbeat at most once per `heartbeat_interval_ms`; the throttle is the
/// broker's, so the caller cannot skip the heartbeat without skipping the
/// clock. The hook driver keeps calling this while the bridge is disabled,
/// so a disabled bridge reads as disabled and never as a dead sim.
#[cfg(any(unix, feature = "dcs-lua"))]
mod tick {
    use core::ffi::{c_int, c_void};

    use crate::lua;

    /// Put `tick` on the table at the top of the stack.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and one stack slot is free.
    pub unsafe fn install(state: *mut c_void) {
        // SAFETY: the push and the setfield pair, leaving the table on top.
        unsafe {
            lua::lua_pushcclosure(state, tick, 0);
            lua::lua_setfield(state, -2, c"tick".as_ptr());
        }
    }

    /// `shim.tick(mission_time)`. Raises before the first `configure`, and
    /// on an argument that is not a finite number: a NaN or an infinity is
    /// a clock that was never read, and stamping it would tell every
    /// consumer the sim is alive at no time at all.
    unsafe extern "C" fn tick(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call. Every raise happens with nothing on the Rust
        // stack that needs dropping.
        unsafe {
            lua::luaL_checktype(state, 1, lua::TNUMBER);
            let mission_time = lua::lua_tonumber(state, 1);
            if !mission_time.is_finite() {
                lua::luaL_argerror(state, 1, c"not a finite number".as_ptr());
                unreachable!("luaL_argerror does not return")
            }
            let bridge = dcsbridge_broker::bridge();
            if !bridge.configured() {
                lua::luaL_error(
                    state,
                    c"configure comes first: no tick can be taken before it".as_ptr(),
                );
                unreachable!("luaL_error does not return")
            }
            bridge.tick(mission_time);
        }
        0
    }
}

/// `shim.epoch(id)` and `shim.epoch(nil)`: the epoch's two boundaries, from
/// the hook driver.
///
/// The hook driver allocates the id at mission load end and publishes it
/// here before it injects the sim driver; at simulation stop it emits
/// `EpochClosed` and then clears it. Between the two the broker stamps
/// every committed record with the id and the mission time, and outside
/// them a record carries neither, which is what a record from a load
/// window is. The call only stores, so it is not refused before the first
/// `configure`.
#[cfg(any(unix, feature = "dcs-lua"))]
mod epoch {
    use core::ffi::{c_int, c_void};

    use crate::lua;

    /// Put `epoch` on the table at the top of the stack.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and one stack slot is free.
    pub unsafe fn install(state: *mut c_void) {
        // SAFETY: the push and the setfield pair, leaving the table on top.
        unsafe {
            lua::lua_pushcclosure(state, epoch, 0);
            lua::lua_setfield(state, -2, c"epoch".as_ptr());
        }
    }

    /// `shim.epoch(id)` opens epoch `id`; `shim.epoch(nil)` and `shim.epoch()`
    /// close the open one. The id is an integer from 1 to 2^32 - 1: zero is
    /// what the field reads as between epochs, so an id of zero would open
    /// an epoch no record could show it was in.
    unsafe extern "C" fn epoch(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call. Every raise happens with nothing on the Rust
        // stack that needs dropping.
        unsafe {
            // No argument reads as LUA_TNONE, which is -1, one below nil.
            let epoch = if lua::lua_type(state, 1) <= lua::TNIL {
                None
            } else {
                lua::luaL_checktype(state, 1, lua::TNUMBER);
                let id = lua::lua_tonumber(state, 1);
                if !(1.0..=f64::from(u32::MAX)).contains(&id) || id.fract() != 0.0 {
                    lua::luaL_argerror(state, 1, c"not an epoch id".as_ptr());
                    unreachable!("luaL_argerror does not return")
                }
                Some(id as u32)
            };
            dcsbridge_broker::bridge().set_epoch(epoch);
        }
        0
    }
}

/// The registration calls: `shim.classes(table)`, `shim.routes(table)`,
/// `shim.caps(table)` and `shim.replies(list)`, from the generated file of
/// either driver.
///
/// Each table maps a topic to a member of the enum its call is named for,
/// spelled by lowercase name or by the schema's number: a class is
/// `'durable'`, `'lossy'`, `'command'` or `'lifecycle'`, a route
/// `'sim_driver'` or `'hook_driver'`, a capability `'read'`, `'command'`
/// or `'reload'`. `replies` takes a list of topics. Each call merges into
/// what the other registrar left and answers the number of rows it added,
/// so a reload's re-registration answers zero and succeeds; a row naming a
/// registered topic with a different value raises, naming the topic and
/// both values, and applies none of the call. A key that is not a string
/// or a value that names no member raises the same way, before the broker
/// is asked. `replies` is its own call rather than an argument of another:
/// ADR 0023.
///
/// The calls only store, so none is refused before the first `configure`.
#[cfg(any(unix, feature = "dcs-lua"))]
mod register {
    use core::ffi::{CStr, c_int, c_void};

    use dcsbridge_broker::registry::{Capability, Member, RecordClass, Target, Topic};

    use crate::lua;

    /// The four calls, by the name each goes on the table under.
    pub const CALLS: [(&CStr, lua::CFunction); 4] = [
        (c"classes", classes),
        (c"routes", routes),
        (c"caps", caps),
        (c"replies", replies),
    ];

    /// Put the four calls on the table at the top of the stack.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and one stack slot is free.
    pub unsafe fn install(state: *mut c_void) {
        // SAFETY: each push and setfield pair, leaving the table on top.
        unsafe {
            for (name, call) in CALLS {
                lua::lua_pushcclosure(state, call, 0);
                lua::lua_setfield(state, -2, name.as_ptr());
            }
        }
    }

    /// The entry every call shares: do the work in a callee that returns,
    /// then raise from a frame that owns nothing. See `push_error`.
    macro_rules! entry {
        ($name:ident, $work:expr) => {
            unsafe extern "C" fn $name(state: *mut c_void) -> c_int {
                // SAFETY: a Lua call. This frame owns nothing.
                unsafe {
                    if $work(state) {
                        return 1;
                    }
                    lua::lua_error(state);
                }
                unreachable!("lua_error does not return")
            }
        };
    }

    entry!(classes, |state| table::<RecordClass>(
        state,
        "classes",
        |rows| { dcsbridge_broker::bridge().register_classes(rows) }
    ));
    entry!(routes, |state| table::<Target>(state, "routes", |rows| {
        dcsbridge_broker::bridge().register_routes(rows)
    }));
    entry!(caps, |state| table::<Capability>(state, "caps", |rows| {
        dcsbridge_broker::bridge().register_caps(rows)
    }));
    entry!(replies, list);

    /// `shim.replies(list)`: the topics a record may be addressed to one
    /// connection on. A set has no value to conflict on, so the broker
    /// never refuses it; a list that is not one of strings is refused
    /// here. Returns whether the count was pushed.
    ///
    /// # Safety
    ///
    /// `state` is inside a Lua call whose first argument is at 1, with
    /// three stack slots free.
    // Never inlined into the entry; see `apply` in `configure`.
    #[inline(never)]
    unsafe fn list(state: *mut c_void) -> bool {
        // SAFETY: the list is read whole before the broker sees any of it.
        unsafe {
            lua::luaL_checktype(state, 1, lua::TTABLE);
            let topics = match read_list(state) {
                Ok(topics) => topics,
                Err(why) => return refuse(state, "replies", why),
            };
            let added = dcsbridge_broker::bridge().register_replies(topics);
            lua::lua_pushinteger(state, added as isize);
        }
        true
    }

    /// One registration call over a table of `V`: read the whole table,
    /// hand it to `register`, and push the rows added or the refusal.
    /// Returns whether the count was pushed.
    ///
    /// # Safety
    ///
    /// `state` is inside a Lua call whose first argument is at 1, with
    /// three stack slots free.
    // Never inlined into the entry; see `apply` in `configure`.
    #[inline(never)]
    unsafe fn table<V: Member>(
        state: *mut c_void,
        call: &'static str,
        register: impl FnOnce(Vec<(Topic, V)>) -> Result<usize, dcsbridge_broker::registry::Conflict>,
    ) -> bool {
        // SAFETY: the caller's contract. The table is read whole before the
        // broker sees any of it, so a bad row leaves the maps as they were.
        unsafe {
            lua::luaL_checktype(state, 1, lua::TTABLE);
            let rows = match read_rows::<V>(state) {
                Ok(rows) => rows,
                Err(why) => return refuse(state, call, why),
            };
            match register(rows) {
                Ok(added) => {
                    lua::lua_pushinteger(state, added as isize);
                    true
                }
                Err(conflict) => refuse(state, call, conflict.to_string()),
            }
        }
    }

    /// Push `<call> refused: <why>` for the entry to raise. Always false.
    unsafe fn refuse(state: *mut c_void, call: &str, why: String) -> bool {
        // SAFETY: the caller's contract.
        unsafe {
            crate::push_error(state, format!("{call} refused: {why}"));
        }
        false
    }

    /// Read the table at argument 1 as topic-to-member rows, or say which
    /// row is not one. Leaves the stack as found.
    ///
    /// # Safety
    ///
    /// `state` is live, argument 1 is a table, and three stack slots are
    /// free.
    unsafe fn read_rows<V: Member>(state: *mut c_void) -> Result<Vec<(Topic, V)>, String> {
        // SAFETY: each `lua_next` pops the key it was given and pushes a
        // pair, both popped before the next call and on the way out of a
        // refusal. The key is checked to be a string before it is read as
        // one, because reading a number key as a string converts it in
        // place and breaks the walk.
        unsafe {
            let mut rows = Vec::new();
            lua::lua_pushnil(state);
            while lua::lua_next(state, 1) != 0 {
                if lua::lua_type(state, -2) != lua::TSTRING {
                    lua::lua_settop(state, -3);
                    return Err("a key is not a topic".into());
                }
                let mut len = 0;
                let s = lua::lua_tolstring(state, -2, &mut len);
                let topic = String::from_utf8_lossy(core::slice::from_raw_parts(s.cast(), len))
                    .into_owned();
                let value = crate::member::<V>(state, -1);
                lua::lua_settop(state, -2);
                match value {
                    Some(value) => rows.push((topic, value)),
                    None => {
                        lua::lua_settop(state, -2);
                        return Err(format!("`{topic}` names no {}", names::<V>()));
                    }
                }
            }
            Ok(rows)
        }
    }

    /// Read the list at argument 1 as topics, or say which entry is not
    /// one. Leaves the stack as found.
    ///
    /// # Safety
    ///
    /// `state` is live, argument 1 is a table, and three stack slots are
    /// free.
    unsafe fn read_list(state: *mut c_void) -> Result<Vec<Topic>, String> {
        // SAFETY: each `lua_next` pops the key it was given and pushes a
        // pair; the value is popped before the next call and both on the
        // way out of a refusal.
        unsafe {
            let mut topics = Vec::new();
            lua::lua_pushnil(state);
            while lua::lua_next(state, 1) != 0 {
                let entry = topics.len() + 1;
                if lua::lua_type(state, -2) != lua::TNUMBER {
                    lua::lua_settop(state, -3);
                    return Err("a key is not a position: the list is a table".into());
                }
                if lua::lua_type(state, -1) != lua::TSTRING {
                    lua::lua_settop(state, -3);
                    return Err(format!("entry {entry} is not a topic"));
                }
                let mut len = 0;
                let s = lua::lua_tolstring(state, -1, &mut len);
                topics.push(
                    String::from_utf8_lossy(core::slice::from_raw_parts(s.cast(), len))
                        .into_owned(),
                );
                lua::lua_settop(state, -2);
            }
            Ok(topics)
        }
    }

    /// The members of `V`, spelled the way a table may spell them, for a
    /// refusal.
    fn names<V: Member>() -> String {
        let names: Vec<&str> = V::ALL.iter().map(|m| m.name()).collect();
        format!("member: one of {}", names.join(", "))
    }
}

/// `shim.poll(target)`: the oldest inbound record on that target's ring, as
/// the connection id it came from, its topic and its bytes, or `nil` when
/// the ring holds nothing.
///
/// Each Lua state polls the ring the route map sends its records to, and
/// under the injection route where the hook driver ferries the sim
/// driver's records it polls both. The id is what `begin_to` takes to
/// address the answer; the topic is what the generated decoders switch
/// on; the bytes are the payload's, opaque here. ADR 0024.
#[cfg(any(unix, feature = "dcs-lua"))]
mod poll {
    use core::ffi::{c_int, c_void};

    use dcsbridge_broker::registry::{Member, Target};

    use crate::lua;

    /// Put `poll` on the table at the top of the stack.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and one stack slot is free.
    pub unsafe fn install(state: *mut c_void) {
        // SAFETY: the push and the setfield pair, leaving the table on top.
        unsafe {
            lua::lua_pushcclosure(state, poll, 0);
            lua::lua_setfield(state, -2, c"poll".as_ptr());
        }
    }

    /// `shim.poll('sim_driver')`, or by the schema's number. Returns three
    /// values, or one `nil`; raises before the first `configure` or on a
    /// target that names no member.
    unsafe extern "C" fn poll(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call. This frame owns nothing, which is what lets
        // it raise; see `push_error`.
        unsafe {
            match take(state) {
                Some(pushed) => pushed,
                None => {
                    lua::lua_error(state);
                    unreachable!("lua_error does not return")
                }
            }
        }
    }

    /// Pop the ring and push what came out, or push the refusal. Returns
    /// how many values were pushed, or `None` for a refusal.
    ///
    /// The record is dropped here, after its parts are copied into Lua,
    /// so nothing this crate allocated is left for the entry to hold.
    ///
    /// # Safety
    ///
    /// `state` is inside a Lua call whose first argument is at 1, with
    /// three stack slots free.
    // Never inlined into the entry; see `apply` in `configure`.
    #[inline(never)]
    unsafe fn take(state: *mut c_void) -> Option<c_int> {
        // SAFETY: the caller's contract. Lua copies every byte it is given.
        unsafe {
            let Some(target) = crate::member::<Target>(state, 1) else {
                let names: Vec<&str> = Target::ALL.iter().map(|t| t.name()).collect();
                crate::push_error(
                    state,
                    format!(
                        "poll refused: the target names no member: one of {}",
                        names.join(", ")
                    ),
                );
                return None;
            };
            match dcsbridge_broker::bridge().poll(target) {
                Ok(Some(command)) => {
                    // An id is numbered from one and a double carries it
                    // exactly to 2^53, which is more connections than a
                    // process will ever accept.
                    lua::lua_pushnumber(state, command.from.get() as f64);
                    lua::lua_pushlstring(state, command.topic.as_ptr().cast(), command.topic.len());
                    lua::lua_pushlstring(state, command.value.as_ptr().cast(), command.value.len());
                    drop(command);
                    Some(3)
                }
                Ok(None) => {
                    lua::lua_pushnil(state);
                    Some(1)
                }
                Err(error) => {
                    crate::push_error(state, format!("poll refused: {error}"));
                    None
                }
            }
        }
    }
}

/// The put calls: one record at a time, built by typed puts into this state's
/// own encoder.
///
/// A record never spans Lua states, so each state gets its own encoder and no
/// lock sits on the put path. The encoder lives behind a userdata every put
/// call closes over, and the userdata's `__gc` drops it with the state. It is
/// allocated by the state's first `begin`, sized by the frame cap in force
/// then, because `configure` comes first and an open allocates nothing.
#[cfg(any(unix, feature = "dcs-lua"))]
mod put {
    use core::ffi::{CStr, c_int, c_void};

    use dcsbridge_broker::encode::{Encoder, Error};
    use dcsbridge_broker::fanout::ConnectionId;
    use dcsbridge_broker::registry::Capability;

    use crate::lua;

    /// The largest connection id a Lua number carries exactly. A number is a
    /// double, exact to 2^53, and an id past that would have lost precision
    /// on its way out of `poll` before it ever came back here.
    const ID_MAX: f64 = 9_007_199_254_740_992.0;

    /// One Lua state's record in progress: the encoder, and where the record
    /// goes when it commits.
    ///
    /// `to` is set by `begin_to`, cleared by `begin`, and taken by `commit`
    /// on every path out of it, so an address never outlives the record it
    /// was given for: a record left open and abandoned cannot mis-address
    /// the next one.
    ///
    /// `encoder` is `None` until the first `begin`, which is refused before
    /// the first `configure`. A put or a commit with no encoder is one with
    /// no record open, and says so.
    ///
    /// `need` is the capability the open record's topic requires, looked up
    /// by the registry check that admitted the `begin`, and handed to the
    /// broker at `commit` so the writer thread can withhold the record from
    /// a connection whose token lacks it without holding a registry. It is
    /// meaningful only while a record is open.
    struct Pending {
        encoder: Option<Encoder>,
        to: Option<ConnectionId>,
        need: Capability,
    }

    /// The calls and their names on the table.
    pub const CALLS: [(&CStr, lua::CFunction); 9] = [
        (c"begin", begin),
        (c"begin_to", begin_to),
        (c"integer", integer),
        (c"double", double),
        (c"string", string),
        (c"boolean", boolean),
        (c"message", message),
        (c"end_message", end_message),
        (c"commit", commit),
    ];

    /// Give the table at the top of the stack an encoder and the calls over it.
    ///
    /// # Safety
    ///
    /// `state` is live, the table is at -1, and three stack slots are free.
    pub unsafe fn install(state: *mut c_void) {
        let pending = Box::into_raw(Box::new(Pending {
            encoder: None,
            to: None,
            need: Capability::Read,
        }));

        // SAFETY: the userdata is exactly one pointer wide and lives as long
        // as the closures that hold it as their upvalue. Every push below is
        // paired with a pop, and the table stays at -2 while the userdata is
        // at -1.
        unsafe {
            let slot =
                lua::lua_newuserdata(state, size_of::<*mut Pending>()).cast::<*mut Pending>();
            slot.write(pending);

            lua::lua_createtable(state, 0, 1);
            lua::lua_pushcclosure(state, gc, 0);
            lua::lua_setfield(state, -2, c"__gc".as_ptr());
            lua::lua_setmetatable(state, -2);

            for (name, call) in CALLS {
                lua::lua_pushvalue(state, -1);
                lua::lua_pushcclosure(state, call, 1);
                lua::lua_setfield(state, -3, name.as_ptr());
            }

            // Drop the userdata off the stack and leave the table on top.
            lua::lua_settop(state, -2);
        }
    }

    /// Drop the encoder when Lua collects its userdata.
    unsafe extern "C" fn gc(state: *mut c_void) -> c_int {
        // SAFETY: `__gc` is called with the userdata as its one argument, and
        // it was written by `install` with a pointer from Box::into_raw.
        unsafe {
            let slot = lua::lua_touserdata(state, 1).cast::<*mut Pending>();
            drop(Box::from_raw(slot.replace(core::ptr::null_mut())));
        }
        0
    }

    /// The calling closure's record in progress.
    ///
    /// # Safety
    ///
    /// `state` is inside a call to one of [`CALLS`], whose first upvalue is
    /// the userdata `install` wrote.
    unsafe fn pending<'a>(state: *mut c_void) -> &'a mut Pending {
        // SAFETY: the caller's contract, and one Lua state runs one call at
        // a time, so no other reference to this record is live.
        unsafe { &mut **lua::lua_touserdata(state, lua::UPVALUE_1).cast::<*mut Pending>() }
    }

    /// The calling closure's encoder, or the error a put gets when no
    /// record is open, since none can be before the first `begin`.
    ///
    /// # Safety
    ///
    /// As [`pending`].
    unsafe fn encoder<'a>(state: *mut c_void) -> Result<&'a mut Encoder, Error> {
        // SAFETY: the caller's contract.
        unsafe { pending(state).encoder.as_mut().ok_or(Error::NotOpen) }
    }

    /// The calling closure's encoder for a `begin`, sized at the frame cap
    /// in force: allocated on the first one, and again on a `begin` that
    /// finds the cap moved, since the cap is live and a buffer at the old
    /// one would refuse records the reader now takes. That is one
    /// allocation per `configure` that changes it, on a record boundary,
    /// and none otherwise. Raises before the first `configure`, which is
    /// what sizes it: a record opened before then would be built against a
    /// default and queued to nothing.
    ///
    /// # Safety
    ///
    /// As [`pending`], and called with nothing on the Rust stack that needs
    /// dropping, because it may raise.
    unsafe fn opening<'a>(state: *mut c_void) -> &'a mut Pending {
        let bridge = dcsbridge_broker::bridge();
        if !bridge.configured() {
            // SAFETY: the caller is a Lua call; the message is static.
            unsafe {
                lua::luaL_error(
                    state,
                    c"configure comes first: no record can be opened before it".as_ptr(),
                );
            }
            unreachable!("luaL_error does not return")
        }
        // SAFETY: the caller's contract.
        let pending = unsafe { pending(state) };
        let cap = bridge.config().max_frame_bytes as usize;
        if pending.encoder.as_ref().is_none_or(|e| e.capacity() != cap) {
            pending.encoder = Some(Encoder::with_capacity(cap));
        }
        pending
    }

    /// The field number at argument 1. Anything outside a field number's
    /// range becomes one the encoder refuses.
    unsafe fn field(state: *mut c_void) -> u32 {
        // SAFETY: the caller is a Lua call, so the state and its stack are live.
        unsafe { lua::luaL_checknumber(state, 1) as u32 }
    }

    /// Raise a Lua error for `error`. Never returns.
    ///
    /// Lua raises with `longjmp`, so this is called with nothing on the Rust
    /// stack that needs dropping.
    unsafe fn raise(state: *mut c_void, error: Error) -> ! {
        let message: &CStr = match error {
            Error::NotOpen => c"no record is open",
            Error::Full => c"the record outgrew its buffer",
            Error::FieldNumber => c"field number outside 1 to 2^29 - 1",
            Error::Depth => c"too many nested messages open",
            Error::Unbalanced => c"message and end_message do not pair",
        };
        // SAFETY: the format string names one argument and one is passed.
        unsafe {
            lua::luaL_error(state, c"%s".as_ptr(), message.as_ptr());
        }
        unreachable!("luaL_error does not return")
    }

    /// Raise unless the put succeeded.
    unsafe fn check(state: *mut c_void, result: Result<(), Error>) -> c_int {
        if let Err(error) = result {
            // SAFETY: the caller is a Lua call.
            unsafe { raise(state, error) }
        }
        0
    }

    /// `shim.begin(topic)`: open a record on the topic for every connection,
    /// discarding and counting one left open. The topic names the record's
    /// type on the wire. Raises before the first `configure`, and on a topic
    /// with no class or no capability registered, counted, with the record
    /// in progress left as it was: the broker holds the schema opaque and
    /// can recover neither value, so a record it could not drop by policy
    /// or keep from the wrong connection is not opened.
    unsafe extern "C" fn begin(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. The topic is
        // an argument, so Lua keeps it alive for the whole call, and Lua
        // stores a string with a terminating NUL, so the pointer serves the
        // error's `%s` as well as the slice. Each raise happens before
        // anything else is held, and the registry check holds no lock by
        // the time it answers.
        unsafe {
            let mut len = 0;
            let s = lua::luaL_checklstring(state, 1, &mut len);
            let topic = core::slice::from_raw_parts(s.cast::<u8>(), len);
            let pending = opening(state);
            let Some(need) = dcsbridge_broker::bridge().registered(topic) else {
                lua::luaL_error(
                    state,
                    c"begin refused: %s has no class or no capability registered".as_ptr(),
                    s,
                );
                unreachable!("luaL_error does not return")
            };
            pending.to = None;
            pending.need = need;
            pending
                .encoder
                .as_mut()
                .expect("opening set it")
                .begin(topic, dcsbridge_broker::bridge().stamp());
        }
        0
    }

    /// `shim.begin_to(conn_id, topic)`: open a record on the topic for one
    /// connection and no other, discarding and counting one left open.
    ///
    /// A separate call rather than a flag on `begin`, so that an address set
    /// by one call and read by another cannot survive an abandoned record and
    /// mis-address the next.
    ///
    /// The id is a number the broker handed out, so it has to be a whole
    /// number from one, and one a double still carries exactly; anything else
    /// is an argument error. A string is refused too, where Lua would have
    /// converted it: an id is never text.
    ///
    /// A topic that is neither a reply nor the acknowledgement is refused,
    /// counted, and raised as an error naming the topic, with no record
    /// opened: the generator only addresses what the schema marks, so the
    /// call is hand-written Lua, and an error at the call site is what tells
    /// its author. A record silently reaching one consumer instead of all of
    /// them would present as missing data at every other. ADR 0017. A reply
    /// with no class or no capability registered is refused the way `begin`
    /// refuses one; ADR 0023.
    unsafe extern "C" fn begin_to(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. Both
        // arguments are Lua's for the whole call, and the topic is a Lua
        // string, which Lua stores with a terminating NUL, so the pointer
        // serves the error's `%s` as well as the slice. The registry check
        // holds no lock by the time it answers, so the raise below jumps
        // past nothing that needs dropping.
        unsafe {
            lua::luaL_checktype(state, 1, lua::TNUMBER);
            let id = lua::lua_tonumber(state, 1);
            if !(1.0..=ID_MAX).contains(&id) || id.fract() != 0.0 {
                lua::luaL_argerror(state, 1, c"not a connection id".as_ptr());
                unreachable!("luaL_argerror does not return")
            }

            let mut len = 0;
            let s = lua::luaL_checklstring(state, 2, &mut len);
            let topic = core::slice::from_raw_parts(s.cast::<u8>(), len);
            if !dcsbridge_broker::bridge().addressable(topic) {
                lua::luaL_error(
                    state,
                    c"begin_to refused: %s is neither a reply nor an acknowledgement".as_ptr(),
                    s,
                );
                unreachable!("luaL_error does not return")
            }
            // A reply is marked addressable by one table and given its
            // class and capability by two others, and needs all three.
            let Some(need) = dcsbridge_broker::bridge().registered(topic) else {
                lua::luaL_error(
                    state,
                    c"begin_to refused: %s has no class or no capability registered".as_ptr(),
                    s,
                );
                unreachable!("luaL_error does not return")
            };

            let pending = opening(state);
            pending
                .encoder
                .as_mut()
                .expect("opening set it")
                .begin(topic, dcsbridge_broker::bridge().stamp());
            pending.to = Some(ConnectionId::from_raw(id as u64));
            pending.need = need;
        }
        0
    }

    /// `shim.integer(field, n)`: a signed 64-bit integer.
    ///
    /// The cast saturates and maps NaN to zero, where C's would be undefined.
    /// A Lua number is a double, so precision is already gone above 2^53.
    unsafe extern "C" fn integer(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe {
            let n = lua::luaL_checknumber(state, 2) as i64;
            check(
                state,
                encoder(state).and_then(|e| e.integer(field(state), n)),
            )
        }
    }

    /// `shim.double(field, x)`.
    unsafe extern "C" fn double(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe {
            let x = lua::luaL_checknumber(state, 2);
            check(
                state,
                encoder(state).and_then(|e| e.double(field(state), x)),
            )
        }
    }

    /// `shim.string(field, str)`: any bytes, copied into the record.
    unsafe extern "C" fn string(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. The string is
        // an argument, so Lua keeps it alive for the whole call.
        unsafe {
            let mut len = 0;
            let s = lua::luaL_checklstring(state, 2, &mut len);
            let bytes = core::slice::from_raw_parts(s.cast::<u8>(), len);
            check(
                state,
                encoder(state).and_then(|e| e.string(field(state), bytes)),
            )
        }
    }

    /// `shim.boolean(field, bool)`: a boolean and nothing coerced to one.
    unsafe extern "C" fn boolean(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe {
            lua::luaL_checktype(state, 2, lua::TBOOLEAN);
            let b = lua::lua_toboolean(state, 2) != 0;
            check(
                state,
                encoder(state).and_then(|e| e.boolean(field(state), b)),
            )
        }
    }

    /// `shim.message(field)`: open a nested message.
    unsafe extern "C" fn message(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe { check(state, encoder(state).and_then(|e| e.message(field(state)))) }
    }

    /// `shim.end_message()`: close the innermost open message.
    unsafe extern "C" fn end_message(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe { check(state, encoder(state).and_then(Encoder::end_message)) }
    }

    /// `shim.commit()`: close the record and queue it, for every connection
    /// or for the one `begin_to` named. Returns true when the record was
    /// queued and false when it was not: a record refused at commit is
    /// discarded and counted, and one the outbound path could not take is
    /// counted there. A commit with no record open is a defect and raises.
    ///
    /// Queued is not delivered. A record queued with no connection attached
    /// is dropped on the writer thread, one addressed to a connection that
    /// has since closed is dropped and counted there, and one a connection's
    /// ring evicts shows there as a gap in `seq`.
    unsafe extern "C" fn commit(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. The tail is
        // copied out of the encoder before the call returns, and nothing
        // else touches the encoder meanwhile.
        unsafe {
            let pending = pending(state);
            // Taken before anything can raise, so the address goes with the
            // record whether or not the record goes anywhere.
            let to = pending.to.take();
            let bridge = dcsbridge_broker::bridge();
            let Some(encoder) = pending.encoder.as_mut() else {
                raise(state, Error::NotOpen)
            };
            let queued = match encoder.commit() {
                Ok(tail) => match to {
                    Some(to) => bridge.commit_to(to, tail).is_ok(),
                    None => bridge.commit(tail).is_ok(),
                },
                Err(Error::NotOpen) => raise(state, Error::NotOpen),
                Err(_) => false,
            };
            lua::lua_pushboolean(state, c_int::from(queued));
        }
        1
    }
}
