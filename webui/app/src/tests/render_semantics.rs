//! Pin Dioxus signal, memo, and callback behavior used by the state architecture. Tests stash handles in thread locals,
//! mutate via `vdom.in_runtime`, then pump bounded renders; nextest isolates each test process.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use dioxus::prelude::*;

use halogen_webui_app_state::{ConnectionState, EpisodeState, SyncStatus};

thread_local! {
    /// Render counter bumped by probe components.
    static RENDERS: Cell<usize> = const { Cell::new(0) };
    /// The root's EpisodeState signal, stashed for the test to drive.
    static STASH: RefCell<Option<Signal<EpisodeState>>> = const { RefCell::new(None) };
    /// The root's ConnectionState signal (the memo-slice test drives this one,
    /// since the `sync_status` slice lives on `ConnectionState`).
    static STASH_CONN: RefCell<Option<Signal<ConnectionState>>> = const { RefCell::new(None) };
}

fn renders() -> usize {
    RENDERS.with(|c| c.get())
}

fn reset_counters() {
    RENDERS.with(|c| c.set(0));
    STASH.with(|s| *s.borrow_mut() = None);
    STASH_CONN.with(|s| *s.borrow_mut() = None);
}

fn stashed_signal() -> Signal<EpisodeState> {
    STASH.with(|s| s.borrow().expect("root stashed its signal"))
}

fn stashed_conn() -> Signal<ConnectionState> {
    STASH_CONN.with(|s| s.borrow().expect("root stashed its connection signal"))
}

/// Pump the vdom until it goes quiet (no work for 100ms) or `budget` passes.
/// Returns the number of passes — a settled tree stops well under any budget;
/// a self-perpetuating dirty loop exhausts it.
async fn pump(vdom: &mut VirtualDom, budget: u32) -> u32 {
    let mut passes = 0;
    while passes < budget {
        match tokio::time::timeout(Duration::from_millis(100), vdom.wait_for_work()).await {
            Ok(()) => {
                vdom.render_immediate(&mut dioxus::core::NoOpMutations);
                passes += 1;
            }
            Err(_) => break,
        }
    }
    passes
}

// ── Signal::set has no PartialEq gate ───────────────────────────────────────
// Why the worker's publish() re-renders every subscriber even when nothing
// changed, and why narrow consumers should sit behind a memo slice.

#[component]
fn SetProbe() -> Element {
    RENDERS.with(|c| c.set(c.get() + 1));
    let sig = use_context::<Signal<EpisodeState>>();
    let n = sig.read().episodes_by_id.len();
    rsx! {
        div { "{n}" }
    }
}

#[component]
fn SetRoot() -> Element {
    let sig = use_signal(EpisodeState::default);
    use_hook(move || {
        STASH.with(|s| *s.borrow_mut() = Some(sig));
        provide_context(sig)
    });
    rsx! {
        SetProbe {}
    }
}

#[tokio::test]
async fn unchanged_set_renotifies_subscribers() {
    reset_counters();
    let mut vdom = VirtualDom::new(SetRoot);
    vdom.rebuild(&mut dioxus::core::NoOpMutations);
    let after_mount = renders();
    assert_eq!(after_mount, 1, "probe mounted once");

    // Write a value-EQUAL EpisodeState: subscribers are still notified.
    let mut sig = stashed_signal();
    vdom.in_runtime(|| sig.set(EpisodeState::default()));
    pump(&mut vdom, 50).await;
    assert!(
        renders() > after_mount,
        "Signal::set gained a PartialEq gate?! The worker's publish() and the \
         EpisodeList store-freshness contract (see webui/sync-engine/src/worker.rs \
         publish) \
         assume unconditional notification — re-audit before relying on this."
    );
}

// ── Memo slices gate notification by PartialEq ──────────────────────────────
// The mechanism behind hooks::use_sync_status and plans/FEATURE_STATE_SLICES.md:
// the memo re-computes on every publish, but its subscribers only re-render
// when the sliced value actually changes.

#[component]
fn MemoProbe(status: Memo<SyncStatus>) -> Element {
    RENDERS.with(|c| c.set(c.get() + 1));
    let s = status();
    rsx! {
        div { "{s:?}" }
    }
}

#[component]
fn MemoRoot() -> Element {
    let sig = use_signal(ConnectionState::default);
    use_hook(move || {
        STASH_CONN.with(|s| *s.borrow_mut() = Some(sig));
        provide_context(sig)
    });
    let status = use_memo(move || sig.read().sync_status.clone());
    rsx! {
        MemoProbe { status }
    }
}

#[tokio::test]
async fn memo_slice_gates_unchanged_set() {
    reset_counters();
    let mut vdom = VirtualDom::new(MemoRoot);
    vdom.rebuild(&mut dioxus::core::NoOpMutations);
    let after_mount = renders();

    // Publish with the slice UNCHANGED (default sync_status is Unknown):
    // memo recomputes, value equal → probe must NOT re-render.
    let mut sig = stashed_conn();
    let unchanged = ConnectionState {
        last_error: Some("noise the slice doesn't read".into()),
        ..Default::default()
    };
    vdom.in_runtime(|| sig.set(unchanged));
    pump(&mut vdom, 50).await;
    assert_eq!(
        renders(),
        after_mount,
        "memo notified subscribers on a value-equal recompute — the slice \
         pattern (use_sync_status, FEATURE_STATE_SLICES) no longer gates"
    );

    // Publish with the slice CHANGED → probe re-renders.
    let changed = ConnectionState {
        sync_status: SyncStatus::Online,
        ..Default::default()
    };
    vdom.in_runtime(|| sig.set(changed));
    pump(&mut vdom, 50).await;
    assert!(
        renders() > after_mount,
        "memo failed to notify subscribers when the sliced value changed"
    );
}

// ── Regression: the /playlists/:id 100%-CPU loop ──────────────────────────── Shape of the original bug: a parent that
// SUBSCRIBES to a signal and passes it as a `ReadSignal<T>` prop alongside a fresh-identity `Callback::new`. The
// derived props memoize value-compares the signal prop; with `EpisodeState: PartialEq` the values compare equal and
// nothing is dirtied. The companion test below shows the loop is real when T is not PartialEq.

#[component]
fn EqChild(state: ReadSignal<EpisodeState>, on_x: Callback<()>) -> Element {
    RENDERS.with(|c| c.set(c.get() + 1));
    let _subscribe = state.read().episodes_by_id.len();
    rsx! {
        div {}
    }
}

#[component]
fn EqParent() -> Element {
    let sig = use_signal(EpisodeState::default);
    use_hook(move || {
        STASH.with(|s| *s.borrow_mut() = Some(sig));
        provide_context(sig)
    });
    // Parent subscribes — the loop ingredient the wedged detail pages had.
    let _subscribe = sig.read().episodes_by_id.len();
    rsx! {
        EqChild {
            state: sig,
            // Deliberately inline: fresh identity per render defeats the
            // exactly-equal early return and forces the signal-compare path.
            on_x: Callback::new(move |_| {}),
        }
    }
}

#[tokio::test]
async fn read_signal_prop_with_fresh_callback_settles() {
    reset_counters();
    let mut vdom = VirtualDom::new(EqParent);
    vdom.rebuild(&mut dioxus::core::NoOpMutations);

    // One real change so the subscribed parent re-renders and re-diffs the child.
    let mut sig = stashed_signal();
    let mut changed = EpisodeState::default();
    changed.episodes_by_podcast.insert(1, vec![10]);
    vdom.in_runtime(|| sig.set(changed));

    let passes = pump(&mut vdom, 50).await;
    assert!(
        passes < 50,
        "subscriber + ReadSignal<EpisodeState> prop + fresh Callback prop did not \
         settle ({passes} passes) — the EpisodeState PartialEq derive stopped \
         protecting props memoization (this is the /playlists/:id wedge)"
    );
}

/// Inverted canary: the SAME shape with a non-PartialEq inner type really does
/// loop — dioxus's memoize can't value-compare it, assumes changed, and
/// `mark_dirty`s the signal the parent subscribes to. This documents WHY
/// `EpisodeState: PartialEq` is load-bearing.
mod non_partialeq_loop {
    use super::*;

    // No PartialEq on purpose.
    #[derive(Clone, Debug)]
    pub struct NoEq(pub u8);

    thread_local! {
        static NOEQ_STASH: RefCell<Option<Signal<NoEq>>> = const { RefCell::new(None) };
    }

    #[component]
    fn LoopChild(state: ReadSignal<NoEq>, on_x: Callback<()>) -> Element {
        let _subscribe = state.read().0;
        rsx! {
            div {}
        }
    }

    #[component]
    fn LoopParent() -> Element {
        let sig = use_signal(|| NoEq(0));
        use_hook(move || NOEQ_STASH.with(|s| *s.borrow_mut() = Some(sig)));
        let _subscribe = sig.read().0;
        rsx! {
            LoopChild {
                state: sig,
                on_x: Callback::new(move |_| {}),
            }
        }
    }

    #[tokio::test]
    async fn non_partialeq_read_signal_prop_loops() {
        let mut vdom = VirtualDom::new(LoopParent);
        vdom.rebuild(&mut dioxus::core::NoOpMutations);

        let mut sig = NOEQ_STASH.with(|s| s.borrow().expect("stashed"));
        vdom.in_runtime(|| sig.set(NoEq(1)));

        let passes = super::pump(&mut vdom, 50).await;
        assert!(
            passes >= 50,
            "expected the non-PartialEq ReadSignal prop to re-render forever \
             (got {passes} passes) — if dioxus's props memoize no longer dirties \
             non-comparable signal props, the EpisodeState PartialEq constraint can \
             be relaxed (and this module deleted)"
        );
    }
}

// ── use_callback identity is stable; inline Callback::new is not ────────────
// Why the EpisodeList pages hoist their callbacks (e.g. the multiselect menu
// openers) into use_callback: stable identity keeps props exactly-equal so
// memoize early-returns.

thread_local! {
    static STABLE_CBS: RefCell<Vec<Callback<()>>> = const { RefCell::new(Vec::new()) };
    static FRESH_CBS: RefCell<Vec<Callback<()>>> = const { RefCell::new(Vec::new()) };
    static TICK: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
}

#[component]
fn CallbackProbe() -> Element {
    let tick = use_signal(|| 0u32);
    use_hook(move || TICK.with(|t| *t.borrow_mut() = Some(tick)));
    let _subscribe = tick.read();

    let stable = use_callback(move |_: ()| {});
    let fresh = Callback::new(move |_: ()| {});
    STABLE_CBS.with(|v| v.borrow_mut().push(stable));
    FRESH_CBS.with(|v| v.borrow_mut().push(fresh));
    rsx! {
        div {}
    }
}

#[tokio::test]
async fn use_callback_identity_stable_across_renders() {
    STABLE_CBS.with(|v| v.borrow_mut().clear());
    FRESH_CBS.with(|v| v.borrow_mut().clear());
    let mut vdom = VirtualDom::new(CallbackProbe);
    vdom.rebuild(&mut dioxus::core::NoOpMutations);

    let mut tick = TICK.with(|t| t.borrow().expect("stashed"));
    for i in 1..=3u32 {
        vdom.in_runtime(|| tick.set(i));
        pump(&mut vdom, 10).await;
    }

    let stable = STABLE_CBS.with(|v| v.borrow().clone());
    let fresh = FRESH_CBS.with(|v| v.borrow().clone());
    assert!(stable.len() >= 4, "probe should have rendered 4+ times");
    assert!(
        stable.windows(2).all(|w| w[0] == w[1]),
        "use_callback handle identity changed across renders — the use_callback \
         hoists on the EpisodeList pages no longer keep props memoized"
    );
    assert!(
        fresh.windows(2).all(|w| w[0] != w[1]),
        "inline Callback::new compared equal across renders — if Callback \
         equality became value-based, the inline-callback prohibition in the \
         pages can be relaxed"
    );
}
