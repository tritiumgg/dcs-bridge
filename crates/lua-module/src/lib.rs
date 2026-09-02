//! `luaopen_dcsbridge`, and the Lua declarations behind it.
//!
//! SPEC 5.1.1 loads the broker by explicit path rather than through `require`:
//!
//! ```lua
//! local path   = lfs.writedir() .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'
//! local loader = assert(package.loadlib(path, 'luaopen_dcsbridge'))
//! local shim   = loader()
//! ```
//!
//! The same call opens the host-native `.so` or `.dylib` a stock Lua 5.1
//! builds against, which is what lets SPEC 17's *Any (native module)* rows run
//! with no DCS present.
//!
//! This crate holds the whole Lua surface and no broker logic. The rings,
//! threads and framing live in `dcsbridge-broker`, which names no Lua symbol
//! and so links into a test binary on any host. ADR 0005.

/// DCS's Lua, or the host's — `build.rs` and the `dcs-lua` feature decide
/// which. ADR 0006.
///
/// The public API is stock Lua 5.1, so these are the stock declarations, and
/// `vendor/lua/lua.def` decides which of the 114 exports the broker may name.
/// `lua_newtable` is a macro over `lua_createtable` and is not an exported
/// symbol, so the real function is named here.
///
/// On Windows the symbols come from `lua.dll` through an import library, so
/// they exist only where that library does. Everywhere else they are left
/// undefined and resolved from the interpreter that opens the module.
#[cfg(any(unix, feature = "dcs-lua"))]
mod lua {
    use core::ffi::{c_char, c_int, c_void};

    unsafe extern "C" {
        /// Push a fresh table sized for `narr` array and `nrec` hash entries.
        pub unsafe fn lua_createtable(state: *mut c_void, narr: c_int, nrec: c_int);

        /// Push `n`. `lua_Integer` is `ptrdiff_t` in a stock 5.1 build.
        pub unsafe fn lua_pushinteger(state: *mut c_void, n: isize);

        /// Push the `len` bytes at `s` as a string, which Lua copies.
        pub unsafe fn lua_pushlstring(state: *mut c_void, s: *const c_char, len: usize);

        /// Pop the top value into `key` of the table at `index`.
        pub unsafe fn lua_setfield(state: *mut c_void, index: c_int, key: *const c_char);
    }
}

/// Open the bridge in `state`, leaving one table on the stack.
///
/// The table is this state's own and the bridge behind it is the process's.
/// Both DCS states load this module, so this runs more than once and each call
/// gets its own table over one set of rings, sockets and registration maps.
/// ADR 0007.
///
/// The table carries the broker version and `opens`, the number of times the
/// module has been opened in this process. The first table reads 1 and the
/// second reads 2, which is how two tables are shown to sit over one bridge.
///
/// # Safety
///
/// `state` must be a live `lua_State` from the Lua that opened this module,
/// with room for two stack slots. It is called by Lua, which guarantees both.
#[cfg(any(unix, feature = "dcs-lua"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaopen_dcsbridge(state: *mut core::ffi::c_void) -> core::ffi::c_int {
    let version = dcsbridge_broker::BROKER_VERSION;
    let opens = dcsbridge_broker::bridge().open();

    // SAFETY: the caller guarantees a live lua_State with two free slots. The
    // table takes one, each pushed value the other, and lua_setfield pops the
    // value back off. Lua copies the bytes it is given, so nothing this crate
    // allocated is left for DCS's C runtime to free.
    unsafe {
        lua::lua_createtable(state, 0, 2);
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
    }

    1
}
