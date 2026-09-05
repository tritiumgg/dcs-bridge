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
        /// built on the Rust side is pushed, dropped, and then raised.
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
pub const INTERFACE_VERSION: &str = "1";

/// Open the bridge in `state`, leaving one table on the stack.
///
/// The table is this state's own and the bridge behind it is the process's.
/// Both DCS states load this module, so this runs more than once and each call
/// gets its own table over one set of rings, sockets and registration maps.
/// ADR 0007.
///
/// The table carries the broker version, the interface version, `opens`, the
/// number of times the module has been opened in this process, `configure`
/// and the put calls. The first table reads 1 and the second reads 2, which
/// is how two tables are shown to sit over one bridge.
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
        lua::lua_createtable(state, 0, 4 + put::CALLS.len() as core::ffi::c_int);
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
    use dcsbridge_broker::state::{Capability, Token};

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
        // SAFETY: a Lua call. The argument is a table for the whole call,
        // and every value pushed while reading it is popped before the read
        // returns, so a raise jumps past nothing on the stack that is this
        // crate's, and past nothing on the heap: the table read is consumed
        // or dropped before the raise, and the message is Lua's by then.
        unsafe {
            lua::luaL_checktype(state, 1, lua::TTABLE);
            let table = match read_table(state) {
                Ok(table) => table,
                Err(why) => raise(state, why),
            };
            match dcsbridge_broker::bridge().configure(table) {
                Ok(applied) => push_applied(state, &applied),
                Err(error) => {
                    // Rendered and dropped before the jump: the error owns
                    // a string the jump would otherwise leak.
                    let why = error.to_string();
                    drop(error);
                    raise(state, why)
                }
            }
        }
        1
    }

    /// Raise `message` as a Lua error. Never returns.
    ///
    /// The message is pushed and dropped before the raise, so the jump
    /// runs no Rust drop it needed to.
    unsafe fn raise(state: *mut c_void, why: String) -> ! {
        let message = format!("configure refused: {why}");
        drop(why);
        // SAFETY: Lua copies the bytes it is given, and the raise takes
        // the copy off the top of the stack. Both strings are dropped by
        // hand before the jump, which runs no drop of its own.
        unsafe {
            lua::lua_pushlstring(state, message.as_ptr().cast(), message.len());
            drop(message);
            lua::lua_error(state);
        }
        unreachable!("lua_error does not return")
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
                let cap = match lua::lua_type(state, -1) {
                    lua::TSTRING => {
                        let mut len = 0;
                        let s = lua::lua_tolstring(state, -1, &mut len);
                        match core::slice::from_raw_parts(s.cast::<u8>(), len) {
                            b"read" => Some(Capability::Read),
                            b"command" => Some(Capability::Command),
                            b"reload" => Some(Capability::Reload),
                            _ => None,
                        }
                    }
                    lua::TNUMBER => {
                        let n = lua::lua_tonumber(state, -1);
                        if n == 1.0 {
                            Some(Capability::Read)
                        } else if n == 2.0 {
                            Some(Capability::Command)
                        } else if n == 3.0 {
                            Some(Capability::Reload)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                lua::lua_settop(state, -2);
                caps.insert(cap?);
            }
            Some(caps)
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
    struct Pending {
        encoder: Option<Encoder>,
        to: Option<ConnectionId>,
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
    /// type on the wire. Whether it is a registered one is a check
    /// registration brings. Raises before the first `configure`.
    unsafe extern "C" fn begin(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. The topic is
        // an argument, so Lua keeps it alive for the whole call, and the
        // raise in `opening` happens before anything else is held.
        unsafe {
            let mut len = 0;
            let s = lua::luaL_checklstring(state, 1, &mut len);
            let topic = core::slice::from_raw_parts(s.cast::<u8>(), len);
            let pending = opening(state);
            pending.to = None;
            pending
                .encoder
                .as_mut()
                .expect("opening set it")
                .begin(topic);
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
    /// them would present as missing data at every other. ADR 0017.
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

            let pending = opening(state);
            pending
                .encoder
                .as_mut()
                .expect("opening set it")
                .begin(topic);
            pending.to = Some(ConnectionId::from_raw(id as u64));
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
