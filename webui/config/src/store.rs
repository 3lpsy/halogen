//! Shared per-user JSON load/save/clear for config and list views. Concrete stores choose a suffix and native filename;
//! web uses namespaced IndexedDB and native uses file helpers.

use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::AccountKey;
use halogen_webui_platform::namespace;

/// A `T` persisted per user at `halogen.{segment}.{suffix}` (web) /
/// `{config_dir}/halogen/{segment}/{file}` (native), where `segment` is the active
/// user's namespace (ambient) or an explicit id (`*_for`). `suffix` is only read on
/// web and `file` only on native, hence the per-target `dead_code` allow on each.
pub(super) struct NamespacedStore<T> {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    suffix: &'static str,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    file: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Serialize + DeserializeOwned + Default> NamespacedStore<T> {
    pub(super) const fn new(suffix: &'static str, file: &'static str) -> Self {
        Self {
            suffix,
            file,
            _marker: PhantomData,
        }
    }

    /// Web: IndexedDB record `kv[suffix]` in `halogen.config.{seg}`.
    /// Native: the JSON file at `{config_dir}/halogen/{seg}/{file}`.
    async fn load_segment(&self, seg: &str) -> T {
        self.try_load_segment(seg).await.unwrap_or_default()
    }

    /// Like `load_segment`, but a backend/parse failure is an `Err`, only a genuinely absent record defaults. The
    /// auth-bearing config store loads through this: collapsing "couldn't read" into `Default` fakes a signed-out
    /// state, and the next save then permanently overwrites the real record.
    async fn try_load_segment(&self, seg: &str) -> Result<T, String> {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::try_load::<T>(seg, self.suffix)
                .await
                .map(|v| v.unwrap_or_default())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            halogen_webui_platform::kv_try_load!(T, "", super::ns_path(seg, self.file))
                .map(|v: Option<T>| v.unwrap_or_default())
        }
    }

    async fn save_segment(&self, seg: &str, value: &T) {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::save(seg, self.suffix, value).await;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            halogen_webui_platform::kv_save!("", super::ns_path(seg, self.file), value);
        }
    }

    async fn clear_segment(&self, seg: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::clear(seg, self.suffix).await;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            halogen_webui_platform::kv_delete!("", super::ns_path(seg, self.file));
        }
    }

    /// Load the active user's value (ambient namespace), default if none.
    pub(super) async fn load(&self) -> T {
        self.load_segment(&namespace::segment()).await
    }

    /// Error-aware [`Self::load`]: absent still defaults, failure is `Err`.
    pub(super) async fn try_load(&self) -> Result<T, String> {
        self.try_load_segment(&namespace::segment()).await
    }

    /// Error-aware per-account load (absent defaults, failure is `Err`) — used
    /// by login/switch, which address an account before the ambient namespace
    /// flips to it.
    pub(super) async fn try_load_for(&self, key: AccountKey) -> Result<T, String> {
        self.try_load_segment(&key.segment()).await
    }

    /// Persist the active user's value.
    pub(super) async fn save(&self, value: &T) {
        self.save_segment(&namespace::segment(), value).await;
    }

    /// Persist a specific account's value.
    pub(super) async fn save_for(&self, key: AccountKey, value: &T) {
        self.save_segment(&key.segment(), value).await;
    }

    /// Remove the active user's value.
    pub(super) async fn clear(&self) {
        self.clear_segment(&namespace::segment()).await;
    }

    /// Remove a specific account's value — reaches a non-active namespace the
    /// ambient `clear` can't (which would otherwise orphan that account's data).
    pub(super) async fn clear_for(&self, key: AccountKey) {
        self.clear_segment(&key.segment()).await;
    }
}
