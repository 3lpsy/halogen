//! `halogen-ui-accounts` — multi-account registry + the account-switch/sign-out
//! orchestration.
//!
//! - `accounts` — the `Accounts` registry + `AccountsStore` persistence
//!   (IndexedDB on web, a JSON file on native) + `StoredAccount` + `jwt_sub`.
//! - `account_actions` — the multi-step `switch_account` / `sign_out_account`
//!   flows (re-mint tokens, flip the active namespace, wipe local data).

pub mod account_actions;
pub mod accounts;
#[cfg(target_arch = "wasm32")]
mod web;
