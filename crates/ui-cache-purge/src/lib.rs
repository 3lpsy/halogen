//! `halogen-ui-cache-purge` — direct browser-storage purges for the
//! `/cache-control` failsafe page. Re-export shell; the implementation lives in
//! [`purge`].

mod purge;

pub use purge::*;
