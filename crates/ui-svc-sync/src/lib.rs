//! The background sync worker. Re-export shell.
//!
//! `SyncService` (the worker that owns the canonical `EpisodeState`, runs the periodic
//! pull/drain loop, and handles `Command`s) lives in [`worker`]; its inherent impl
//! is split across the sibling modules (`connectivity`/`data_ops`/`network`), which
//! reach the worker internals via `super::`. `download`/`reorder`/`tasks` are
//! detached free-function helpers (they take what they need as args and report back
//! through the `Command` channel). `poll_job` is the admin poll orchestration
//! (+ `PullPhase`).

mod connectivity;
mod data_ops;
mod download;
mod network;
pub mod poll_job;
mod reorder;
mod runtime;
mod tasks;
mod tracked;
#[cfg(target_arch = "wasm32")]
mod web_runtime;
mod worker;

pub use halogen_ui_commands::{Command, RedactedToken};
pub use runtime::{
    BackgroundRuntime, FromWorker, ToWorker, WorkerEvent, WorkerSeam, WorkerToasts, worker_seam,
};
#[cfg(target_arch = "wasm32")]
pub use web_runtime::WebWorkerRuntime;
pub use worker::SyncService;

// Worker internals the sibling-module impls reach via `super::` (crate-private).
pub(crate) use download::device_download_task;
pub(crate) use worker::{
    DEGRADED_RTT_MS, InFlight, PULL_PAGE_SIZE, RTT_WINDOW, plural, sleep_secs, spawn_task,
};
