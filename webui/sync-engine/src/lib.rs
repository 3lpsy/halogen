//! SyncService owns commands, working state, and periodic pull/drain across sibling impl modules. Detached
//! download/reorder/task helpers report through commands; poll_job handles admin polling.

mod connectivity;
mod data_ops;
mod delta;
mod download;
mod network;
pub mod poll_job;
mod rejection;
mod reorder;
mod runtime;
mod tasks;
mod tracked;
mod transaction;
#[cfg(target_arch = "wasm32")]
mod web_runtime;
mod worker;

pub use halogen_webui_commands::{Command, RedactedToken};
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
