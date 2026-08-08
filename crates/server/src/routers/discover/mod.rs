//! Online-only podcast discovery endpoints.
//!
//! - `GET /api/v1/discover/search?q=<term>&providers[]=itunes&providers[]=gpodder`
//!   — federated search; partial results + per-provider errors, never artwork.
//! - `GET /api/v1/discover/providers` — which providers the UI can toggle.
//!
//! Both are mounted in `protected_routes`, so they require a valid bearer token
//! like the rest of the API. All outbound provider calls happen server-side
//! (CSP: the client only ever talks to our origin).

pub mod providers;
pub mod search;
