//! Tiny JSON key/value persistence for the **native** client-side stores: a JSON
//! file per (namespaced) value. The web stores are on IndexedDB
//! (`halogen-ui-idb`), so these macros are invoked only from native-`cfg` code; the
//! `web_key` argument is accepted (so call sites can pass both) but ignored.
//!
//! Why macros (not functions): the native path is built from `directories`-based
//! helpers that don't compile on `wasm32`. A macro keeps the native-path
//! expression inside the macro body, so it's never compiled for web; a function
//! taking the path as an argument would force the caller to build it on both
//! targets. The `web_key` argument is still accepted positionally (so call sites
//! can pass the same pair on both targets) but is matched and dropped — never
//! emitted, so a wasm-only `web_key` const is never referenced on native.

/// `kv_load_opt!(Type, web_key, native_path)` → `Option<Type>` (`None` when
/// absent/unreadable). Reads the JSON file at `native_path` (native only).
#[macro_export]
macro_rules! kv_load_opt {
    ($ty:ty, $web_key:expr, $native_path:expr $(,)?) => {{
        ::std::fs::read_to_string($native_path)
            .ok()
            .and_then(|s| ::serde_json::from_str::<$ty>(&s).ok())
    }};
}

/// `kv_try_load!(Type, web_key, native_path)` → `Result<Option<Type>, String>`:
/// `Ok(None)` only for a genuinely ABSENT file; a read failure or corrupt JSON
/// is an `Err`. For values where "couldn't read" must not silently become "use
/// defaults" — a defaulted auth config, later saved, permanently overwrites the
/// real one (`kv_load_opt!` collapses all three cases and is only safe for
/// re-derivable preferences).
#[macro_export]
macro_rules! kv_try_load {
    ($ty:ty, $web_key:expr, $native_path:expr $(,)?) => {{
        let path = $native_path;
        match ::std::fs::read_to_string(&path) {
            Err(e) if e.kind() == ::std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("read {}: {e}", path.display())),
            Ok(s) => match ::serde_json::from_str::<$ty>(&s) {
                Ok(v) => Ok(Some(v)),
                Err(e) => Err(format!("parse {}: {e}", path.display())),
            },
        }
    }};
}

/// `kv_save!(web_key, native_path, &value)` — persist a JSON value (creating the
/// parent directory). Serialization/IO errors are ignored.
#[macro_export]
macro_rules! kv_save {
    ($web_key:expr, $native_path:expr, $value:expr $(,)?) => {{
        let path = $native_path;
        if let Some(parent) = path.parent() {
            let _ = ::std::fs::create_dir_all(parent);
        }
        if let Ok(json) = ::serde_json::to_string($value) {
            let _ = ::std::fs::write(&path, json);
        }
    }};
}

/// `kv_delete!(web_key, native_path)` — remove a stored value.
#[macro_export]
macro_rules! kv_delete {
    ($web_key:expr, $native_path:expr $(,)?) => {{
        let _ = ::std::fs::remove_file($native_path);
    }};
}
