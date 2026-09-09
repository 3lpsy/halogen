//! In-app toast notifications. Re-export shell. The reactive [`queue`] (`ToastQueue`/`ToastHandle`/`Toast`/
//! `ToastLevel` + `ApiResultExt`) and the [`classify`] error funnel (`ApiError` -> `ToastDecision` under a
//! `ToastPolicy`) live in their own modules.

mod classify;
mod queue;

pub use classify::{
    ToastDecision, ToastPolicy, classify, is_countable_failure, is_permanent_failure,
};
pub use queue::*;
