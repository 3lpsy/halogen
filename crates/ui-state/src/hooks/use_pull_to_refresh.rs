//! Pull-to-refresh gesture for the list scroll containers.
//!
//! Two-stage interaction, attached to a scroll container by id:
//! 1. **Pull down + release** → a client-side refresh (the caller's `on_refresh`,
//!    which re-fetches the list with the current sort/filter/search).
//! 2. **Pull down + hold** past the threshold → a 5→…→1 countdown; held to 0 (and
//!    only for admins) it starts a **server poll job**, polls its status, then
//!    refreshes the list and toasts a summary.
//!
//! The gesture itself lives in a small JS shim ([`PTR_JS`]) attached to the
//! container — it owns scroll-position checks, the countdown timer, and
//! axis detection (so a horizontal row-swipe never arms a pull), and it ignores
//! gestures that start on a reorder grip (`[data-reorder-grip]`, so dragging a
//! playlist row to reorder never arms a pull) — and reports high-level messages
//! back over the `dioxus.send`/`eval.recv` bridge. Rust only tracks the visual
//! [`PullPhase`]; the poll orchestration lives in [`halogen_ui_svc_sync::poll_job`].

use dioxus::prelude::*;
use serde::Deserialize;

use crate::hooks::{use_config, use_is_admin, use_toast};
use halogen_ui_platform::time::sleep_ms;
use halogen_ui_svc_sync::poll_job::run_poll_job;
// `PullPhase` lives in `halogen-ui-svc-sync` (next to the poll orchestration) so both
// it and this gesture hook can name it across the crate boundary.
pub use halogen_ui_svc_sync::poll_job::PullPhase;

use super::use_dom_stream::use_dom_stream;

/// Pixels the user must pull before the gesture arms. Keep in sync with `PTR_JS`.
pub const PULL_THRESHOLD: f64 = 64.0;

/// Handle returned by [`use_pull_to_refresh`] — just the phase for the indicator.
#[derive(Clone, Copy)]
pub struct PullToRefresh {
    pub phase: Signal<PullPhase>,
}

/// A message from the JS gesture shim.
#[derive(Deserialize)]
struct PullMsg {
    kind: String,
    #[serde(default)]
    offset: f64,
    #[serde(default)]
    count: i32,
}

/// Attach pull-to-refresh to the scroll container with id `container_id`.
///
/// `scope` is read when a poll fires: `Some(podcast_id)` scopes the server poll
/// to one feed (podcast-detail), `None` polls all feeds. `on_refresh` resets the
/// list's paging cursors so the existing effects re-fetch.
pub fn use_pull_to_refresh(
    container_id: &'static str,
    scope: Memo<Option<i32>>,
    on_refresh: Callback<()>,
) -> PullToRefresh {
    let mut phase = use_signal(|| PullPhase::Idle);
    let is_admin = use_is_admin();
    let config = use_config();
    let toast = use_toast();

    // Route the gesture shim through `use_dom_stream` so its listeners + countdown
    // interval are torn down on unmount (via the harness AbortController + `use_drop`),
    // rather than parking a bare `document::eval` whose `setInterval`/listeners would
    // leak per list navigation.
    use_dom_stream(
        move || PTR_JS.replace("__CONTAINER__", container_id),
        move |msg: PullMsg| match msg.kind.as_str() {
            "pull" => {
                if !phase.peek().is_busy() {
                    phase.set(PullPhase::Pulling(msg.offset));
                }
            }
            "armed" => {
                if !phase.peek().is_busy() {
                    phase.set(PullPhase::Armed(msg.count));
                }
            }
            "reset" => {
                if !phase.peek().is_busy() {
                    phase.set(PullPhase::Idle);
                }
            }
            "refresh" => {
                phase.set(PullPhase::Refreshing);
                on_refresh.call(());
                // The list shows its own spinner while re-fetching; clear the
                // pull indicator after a beat unless a new gesture took over.
                spawn(async move {
                    sleep_ms(700).await;
                    if *phase.peek() == PullPhase::Refreshing {
                        phase.set(PullPhase::Idle);
                    }
                });
            }
            // Held to the bottom of the countdown. The admin/non-admin branch is
            // decided HERE (reading the reactive `is_admin` memo at gesture time),
            // not baked into the gesture JS at mount — so a post-login admin flip
            // takes effect without remounting the list. Admins kick a server poll
            // job; everyone else just refreshes the list (same as a release-armed
            // pull).
            "autofire" => {
                if is_admin() {
                    phase.set(PullPhase::Polling);
                    let podcast_id = *scope.peek();
                    spawn(async move {
                        run_poll_job(config, toast, podcast_id, on_refresh, phase).await;
                    });
                } else {
                    phase.set(PullPhase::Refreshing);
                    on_refresh.call(());
                    spawn(async move {
                        sleep_ms(700).await;
                        if *phase.peek() == PullPhase::Refreshing {
                            phase.set(PullPhase::Idle);
                        }
                    });
                }
            }
            _ => {}
        },
    );

    PullToRefresh { phase }
}

/// Gesture shim attached to the scroll container. `__CONTAINER__` is substituted
/// before eval. Reports `pull`/`armed`/`refresh`/`autofire`/`reset`. The admin vs.
/// non-admin decision for `autofire` (held-to-zero) is made in Rust off the live
/// `is_admin` memo, not baked in here, so it can't go stale after a login flip.
///
/// Run through the [`use_dom_stream`] teardown harness: every listener is
/// registered against the harness-provided `signal` (an `AbortSignal`) and the
/// countdown interval is cleared on `abort`, so the listeners + timer are torn down
/// on unmount (consistent with `use_scroll_memory`) — no `ptrBound` dataset guard
/// or leaked `setInterval` per list navigation.
///
/// Why touch events (not Pointer events): on iOS Safari `preventDefault()` on a
/// `pointermove` does NOT cancel scrolling, so the native rubber-band still runs
/// and Safari fires `pointercancel` the moment it starts scrolling — the gesture
/// dies and the indicator never shows (the exact bug seen on-device). Only a
/// NON-passive `touchmove` + `preventDefault()` suppresses the rubber-band and
/// keeps the move stream alive. We use touch events for mobile (and Chrome's
/// device-emulation, which dispatches touch), and a parallel mouse path for the
/// desktop pointer. Both feed one shared state machine.
///
/// We only `preventDefault()` once we've decided the gesture is a downward pull
/// from the very top — horizontal moves (row swipe-to-action) and upward scrolls
/// pass straight through to native handling.
const PTR_JS: &str = r#"
const root = document.getElementById('__CONTAINER__');
if (root) {
  const THRESHOLD = 64;
  const MAXP = 140;
  const DEADZONE = 4;
  let startX = null, startY = null, decided = false, owning = false, pulling = false,
      armed = false, timer = null, count = 0;

  function clearTimer() { if (timer) { clearInterval(timer); timer = null; } }
  // On unmount the harness aborts `signal`, which removes every listener below and
  // clears any live countdown interval — no parked timer/listeners leak per nav.
  signal.addEventListener('abort', clearTimer);
  function reset() {
    startX = null; startY = null; decided = false; owning = false;
    pulling = false; armed = false; count = 0; clearTimer();
  }

  function onStart(x, y) {
    // Multiselect mode owns the list's gestures — never arm a pull/poll then.
    if (root.dataset.multiselect === '1') { reset(); return; }
    // Only arm when the list is scrolled to the very top.
    if (root.scrollTop > 0) { reset(); return; }
    startX = x; startY = y; decided = false; owning = false;
    pulling = false; armed = false; count = 0; clearTimer();
  }

  // Returns true when the caller should preventDefault (we own — or might be
  // about to own — a downward pull from the top).
  function onMove(x, y) {
    if (startY === null) { return false; }
    const dx = x - startX;
    const dy = y - startY;
    if (!decided) {
      // Below the deadzone we can't tell the axis yet. Pre-emptively claim any
      // downward drift so iOS doesn't lock the touch to native scrolling before
      // we get to decide (it ignores later preventDefault once it has committed).
      if (Math.abs(dx) < DEADZONE && Math.abs(dy) < DEADZONE) { return dy > 0; }
      decided = true;
      // Horizontal (row swipe) or upward (scroll into the list) → not ours.
      if (Math.abs(dx) > Math.abs(dy) || dy <= 0) { startY = null; return false; }
      owning = true;
    }
    if (!owning) { return false; }
    // Content scrolled under us somehow → bail back to native.
    if (root.scrollTop > 0) { reset(); dioxus.send({ kind: 'reset' }); return false; }
    if (dy <= 0) {
      // Pulled back up to the top: collapse the reveal but keep owning so a
      // re-pull doesn't hand control to a native scroll.
      if (armed) { armed = false; clearTimer(); }
      pulling = true;
      dioxus.send({ kind: 'pull', offset: 0 });
      return true;
    }
    pulling = true;
    const offset = Math.min(dy, MAXP);
    if (dy >= THRESHOLD) {
      if (!armed) {
        armed = true; count = 5;
        dioxus.send({ kind: 'armed', count: count });
        timer = setInterval(() => {
          count -= 1;
          if (count > 0) {
            dioxus.send({ kind: 'armed', count: count });
          } else {
            clearTimer();
            // Fire while the finger is still down (the auto-poll/refresh case);
            // clear gesture state but leave the phase to Rust.
            startX = null; startY = null; decided = false; owning = false;
            pulling = false; armed = false;
            // Held to zero — Rust decides poll (admin) vs. refresh off the live memo.
            dioxus.send({ kind: 'autofire' });
          }
        }, 1000);
      }
    } else {
      if (armed) { armed = false; clearTimer(); }
      dioxus.send({ kind: 'pull', offset: offset });
    }
    return true;
  }

  function onEnd() {
    // Auto-fired already (timer cleared startY): leave Rust's phase alone.
    if (startY === null && !armed) { reset(); return; }
    const wasArmed = armed;
    const wasPulling = pulling;
    reset();
    if (wasArmed) {
      dioxus.send({ kind: 'refresh' });   // released past the threshold
    } else if (wasPulling) {
      dioxus.send({ kind: 'reset' });      // short tug → collapse the reveal
    }
  }

  // ── Touch (mobile + device emulation) ──────────────────────────────────────
  // Ignore a gesture that begins on a reorder grip — the grip runs its own
  // vertical drag-to-reorder (see episode_list/item.rs) and its touchstart
  // bubbles up here; without this, dragging a row down from the top would also
  // arm a pull-to-refresh / poll.
  function onGrip(e) {
    const t = e.target;
    return !!(t && t.closest && t.closest('[data-reorder-grip]'));
  }

  root.addEventListener('touchstart', (e) => {
    if (e.touches.length !== 1) { reset(); return; }
    if (onGrip(e)) { reset(); return; }
    onStart(e.touches[0].clientX, e.touches[0].clientY);
  }, { passive: true, signal });
  root.addEventListener('touchmove', (e) => {
    if (startY === null || !e.touches.length) { return; }
    const t = e.touches[0];
    if (onMove(t.clientX, t.clientY) && e.cancelable) { e.preventDefault(); }
  }, { passive: false, signal });
  root.addEventListener('touchend', onEnd, { signal });
  root.addEventListener('touchcancel', onEnd, { signal });

  // ── Mouse (desktop) ────────────────────────────────────────────────────────
  let mouseDown = false;
  root.addEventListener('mousedown', (e) => {
    if (e.button !== 0) { return; }
    if (onGrip(e)) { return; }
    mouseDown = true;
    onStart(e.clientX, e.clientY);
  }, { signal });
  root.addEventListener('mousemove', (e) => {
    if (!mouseDown) { return; }
    // Suppress text selection while dragging the list down.
    if (onMove(e.clientX, e.clientY)) { e.preventDefault(); }
  }, { signal });
  function endMouse() { if (mouseDown) { mouseDown = false; onEnd(); } }
  root.addEventListener('mouseup', endMouse, { signal });
  root.addEventListener('mouseleave', endMouse, { signal });
}
"#;
