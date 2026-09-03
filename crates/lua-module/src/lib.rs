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

    /// The type tag of a boolean, for `luaL_checktype`.
    pub const TBOOLEAN: c_int = 1;

    unsafe extern "C" {
        /// Push a fresh table sized for `narr` array and `nrec` hash entries.
        pub unsafe fn lua_createtable(state: *mut c_void, narr: c_int, nrec: c_int);

        /// Push `n`. `lua_Integer` is `ptrdiff_t` in a stock 5.1 build.
        pub unsafe fn lua_pushinteger(state: *mut c_void, n: isize);

        /// Push the `len` bytes at `s` as a string, which Lua copies.
        pub unsafe fn lua_pushlstring(state: *mut c_void, s: *const c_char, len: usize);

        /// Push nil.
        pub unsafe fn lua_pushnil(state: *mut c_void);

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

        /// The string at `narg` with its length in `len`, or raise.
        pub unsafe fn luaL_checklstring(
            state: *mut c_void,
            narg: c_int,
            len: *mut usize,
        ) -> *const c_char;

        /// Raise unless the value at `narg` has type `t`.
        pub unsafe fn luaL_checktype(state: *mut c_void, narg: c_int, t: c_int);

        /// The truth of the value at `index`.
        pub unsafe fn lua_toboolean(state: *mut c_void, index: c_int) -> c_int;

        /// Raise an error formatted like `printf`. Never returns.
        pub unsafe fn luaL_error(state: *mut c_void, fmt: *const c_char, ...) -> c_int;
    }
}

/// Open the bridge in `state`, leaving one table on the stack.
///
/// The table is this state's own and the bridge behind it is the process's.
/// Both DCS states load this module, so this runs more than once and each call
/// gets its own table over one set of rings, sockets and registration maps.
/// ADR 0007.
///
/// The table carries the broker version, `opens`, the number of times the
/// module has been opened in this process, and the put calls. The first table
/// reads 1 and the second reads 2, which is how two tables are shown to sit
/// over one bridge.
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
        lua::lua_createtable(state, 0, 2 + put::CALLS.len() as core::ffi::c_int);
        lua::lua_pushlstring(
            state,
            version.as_ptr().cast::<core::ffi::c_char>(),
            version.len(),
        );
        // -1 is the version string just pushed, so the table is at -2.
        lua::lua_setfield(state, -2, c"version".as_ptr());

        // A count that outgrew isize would need more Lua states than DCS has.
        lua::lua_pushinteger(state, opens as isize);
        lua::lua_setfield(state, -2, c"opens".as_ptr());

        put::install(state);
    }

    1
}

/// The put calls: one record at a time, built by typed puts into this state's
/// own encoder.
///
/// A record never spans Lua states, so each state gets its own encoder and no
/// lock sits on the put path. The encoder lives behind a userdata every put
/// call closes over, and the userdata's `__gc` drops it with the state.
#[cfg(any(unix, feature = "dcs-lua"))]
mod put {
    use core::ffi::{CStr, c_int, c_void};

    use dcsbridge_broker::encode::{Encoder, Error};

    use crate::lua;

    /// The most bytes one record may hold: the default frame cap, until the
    /// first `configure` supplies the configured value and allocates then.
    const RECORD_BYTES: usize = 1 << 20;

    /// The calls and their names on the table.
    pub const CALLS: [(&CStr, lua::CFunction); 8] = [
        (c"begin", begin),
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
        let encoder = Box::into_raw(Box::new(Encoder::with_capacity(RECORD_BYTES)));

        // SAFETY: the userdata is exactly one pointer wide and lives as long
        // as the closures that hold it as their upvalue. Every push below is
        // paired with a pop, and the table stays at -2 while the userdata is
        // at -1.
        unsafe {
            let slot =
                lua::lua_newuserdata(state, size_of::<*mut Encoder>()).cast::<*mut Encoder>();
            slot.write(encoder);

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
            let slot = lua::lua_touserdata(state, 1).cast::<*mut Encoder>();
            drop(Box::from_raw(slot.replace(core::ptr::null_mut())));
        }
        0
    }

    /// The calling closure's encoder.
    ///
    /// # Safety
    ///
    /// `state` is inside a call to one of [`CALLS`], whose first upvalue is
    /// the userdata `install` wrote.
    unsafe fn encoder<'a>(state: *mut c_void) -> &'a mut Encoder {
        // SAFETY: the caller's contract, and one Lua state runs one call at
        // a time, so no other reference to this encoder is live.
        unsafe { &mut **lua::lua_touserdata(state, lua::UPVALUE_1).cast::<*mut Encoder>() }
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

    /// `shim.begin(topic)`: open a record on the topic, discarding and
    /// counting one left open. The topic names the record's type on the wire.
    /// Whether it is a registered one is a check registration brings.
    unsafe extern "C" fn begin(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. The topic is
        // an argument, so Lua keeps it alive for the whole call.
        unsafe {
            let mut len = 0;
            let s = lua::luaL_checklstring(state, 1, &mut len);
            let topic = core::slice::from_raw_parts(s.cast::<u8>(), len);
            encoder(state).begin(topic);
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
            check(state, encoder(state).integer(field(state), n))
        }
    }

    /// `shim.double(field, x)`.
    unsafe extern "C" fn double(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe {
            let x = lua::luaL_checknumber(state, 2);
            check(state, encoder(state).double(field(state), x))
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
            check(state, encoder(state).string(field(state), bytes))
        }
    }

    /// `shim.boolean(field, bool)`: a boolean and nothing coerced to one.
    unsafe extern "C" fn boolean(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe {
            lua::luaL_checktype(state, 2, lua::TBOOLEAN);
            let b = lua::lua_toboolean(state, 2) != 0;
            check(state, encoder(state).boolean(field(state), b))
        }
    }

    /// `shim.message(field)`: open a nested message.
    unsafe extern "C" fn message(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe { check(state, encoder(state).message(field(state))) }
    }

    /// `shim.end_message()`: close the innermost open message.
    unsafe extern "C" fn end_message(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built.
        unsafe { check(state, encoder(state).end_message()) }
    }

    /// `shim.commit()`: close the record and return its body as a string,
    /// until the commit ring takes it instead. A record refused at commit is
    /// discarded and counted, and the call returns nil. A commit with no
    /// record open is a defect and raises.
    unsafe extern "C" fn commit(state: *mut c_void) -> c_int {
        // SAFETY: a Lua call over the closure `install` built. The body is
        // pushed before anything else touches the encoder, and Lua copies it.
        unsafe {
            match encoder(state).commit() {
                Ok(body) => lua::lua_pushlstring(state, body.as_ptr().cast(), body.len()),
                Err(Error::NotOpen) => raise(state, Error::NotOpen),
                Err(_) => lua::lua_pushnil(state),
            }
        }
        1
    }
}
