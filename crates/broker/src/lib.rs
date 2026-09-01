//! The DCS-Bridge broker.
//!
//! A stub. The rings, threads and framed TCP transport come with Phase 2.

/// The broker build, carried in the handshake for bug reports.
///
/// SPEC 13.3 compares nothing against it.
pub const BROKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// DCS's Lua, reached through the import library `build.rs` generates from
/// `vendor/lua/lua.def`. SPEC 5.1.1.
///
/// The public API is stock Lua 5.1, so these are the stock declarations. The
/// `.def` decides which of the 114 exports the broker may name.
#[cfg(all(feature = "dcs-lua", target_os = "windows"))]
mod lua {
    use core::ffi::{c_int, c_void};

    unsafe extern "C" {
        /// Index of the top element of `state`'s stack, which is its depth.
        pub unsafe fn lua_gettop(state: *mut c_void) -> c_int;
    }
}

/// Depth of `state`'s stack.
///
/// One call into `lua.dll`, so the linker resolves a symbol through the import
/// library and the binding shows up in the built DLL's import table. It goes
/// away once the broker exports `luaopen_dcsbridge`.
///
/// # Safety
///
/// `state` must be a live `lua_State` from the Lua that loaded this DLL.
#[cfg(all(feature = "dcs-lua", target_os = "windows"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dcsbridge_lua_stack_depth(
    state: *mut core::ffi::c_void,
) -> core::ffi::c_int {
    // SAFETY: the caller guarantees a live lua_State, and `lua_gettop` reads
    // one field of it. Nothing crosses the C runtime boundary, which is the
    // constraint SPEC 5.1.1 puts on every call into DCS's Lua.
    unsafe { lua::lua_gettop(state) }
}
