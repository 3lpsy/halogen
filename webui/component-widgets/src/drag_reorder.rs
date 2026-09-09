//! Engage reorder only after HOLD_MS with movement within MOVE_CANCEL_PX; earlier scrolling/cancellation abandons it.
//! Snapshot geometry at engagement, then prevent touch scrolling until release. Report the target row's original
//! index_attr for remove-then-insert ordering; JS keeps tracking outside the grip.

use dioxus::prelude::*;

/// How long the finger must rest on the grip before a reorder engages (ms). Below
/// this a movement is treated as a scroll, not a drag.
const HOLD_MS: u32 = 250;
/// Movement (px, either axis) during the hold window that abandons the reorder as
/// a scroll/flick.
const MOVE_CANCEL_PX: f64 = 8.0;

/// Run a press-and-hold drag-to-reorder gesture for one row. `container_id` is the scroll container's dom id;
/// `index_attr` is the data-attribute each row carries with its index (e.g. `"data-ep-index"`); `card_id` is the
/// dragged row's dom id; `dragged_index` / `start_y` come from the pointer-down event. `on_drop(target)` fires only
/// when the row moved (the reported index is valid and differs from `dragged_index`).
pub fn start_drag_reorder(
    container_id: &str,
    index_attr: &str,
    card_id: &str,
    dragged_index: usize,
    start_y: f64,
    on_drop: impl FnOnce(usize) + 'static,
) {
    let hold_ms = HOLD_MS;
    let move_cancel = MOVE_CANCEL_PX;
    let script = format!(
        r#"
        const startY = {start_y};
        const draggedIndex = {dragged_index};
        const HOLD_MS = {hold_ms};
        const MOVE_CANCEL = {move_cancel};
        const container = document.getElementById('{container_id}');
        const card = document.getElementById('{card_id}');
        if (!card || !container) {{ dioxus.send(-1); }} else {{
            let engaged = false;
            let done = false;
            let holdTimer = null;
            let startX = null;          // captured on the first move (pointerdown gives only Y)
            let rows, rects, pitch, target = draggedIndex;
            // Non-passive so it can cancel scroll once the drag owns the gesture.
            const blockScroll = (e) => {{ if (e.cancelable) e.preventDefault(); }};
            const teardown = () => {{
                window.removeEventListener('pointermove', onMove);
                window.removeEventListener('pointerup', onUp);
                window.removeEventListener('pointercancel', onCancel);
                window.removeEventListener('touchmove', blockScroll);
                if (holdTimer !== null) {{ clearTimeout(holdTimer); holdTimer = null; }}
            }};
            const finish = (result) => {{
                if (done) return;
                done = true;
                if (engaged) {{
                    rows.forEach(r => {{ r.style.transition = ''; r.style.transform = ''; }});
                    card.style.zIndex = ''; card.style.position = '';
                    card.style.transform = ''; card.style.transition = ''; card.style.boxShadow = '';
                }}
                teardown();
                dioxus.send(result);
            }};
            // Open a one-row gap at target T by sliding the rows between the origin
            // and T past the vacated slot.
            const layout = (T) => {{
                rows.forEach(r => {{
                    if (r === card) return;
                    const i = parseInt(r.getAttribute('{index_attr}'));
                    let shift = 0;
                    if (T > draggedIndex && i > draggedIndex && i <= T) shift = -pitch;
                    else if (T < draggedIndex && i >= T && i < draggedIndex) shift = pitch;
                    r.style.transform = shift ? 'translateY(' + shift + 'px)' : '';
                }});
            }};
            // The hold completed without a scroll: lift the row and snapshot
            // geometry NOW (the list may have scrolled during the hold).
            const engage = () => {{
                holdTimer = null;
                engaged = true;
                rows = Array.from(container.querySelectorAll('[{index_attr}]'));
                rects = new Map();
                rows.forEach(r => rects.set(r, r.getBoundingClientRect()));
                // Displaced rows slide by the space the dragged row frees: its own
                // height plus any inter-row gap (rows differ in height when titles
                // wrap, so don't assume a uniform pitch).
                let gap = 0;
                if (rows.length >= 2) {{
                    const g = rects.get(rows[1]).top - (rects.get(rows[0]).top + rects.get(rows[0]).height);
                    if (g > 0) gap = g;
                }}
                // Reuse the card's rect from the batch above (it's one of `rows`)
                // instead of a second `getBoundingClientRect()` — that extra read,
                // after the loop, would force another reflow for the same geometry.
                // Fall back to a direct measure if the card somehow wasn't in `rows`.
                pitch = (rects.get(card) || card.getBoundingClientRect()).height + gap;
                rows.forEach(r => {{ if (r !== card) r.style.transition = 'transform 120ms ease'; }});
                card.style.zIndex = '50';
                card.style.position = 'relative';
                card.style.transition = 'transform 80ms ease';
                card.style.boxShadow = '0 8px 24px rgba(0,0,0,0.25)';   // lift cue
                // Lock scrolling for the drag (grip is pan-y, so touch-action alone
                // wouldn't stop a mid-gesture scroll).
                window.addEventListener('touchmove', blockScroll, {{ passive: false }});
            }};
            const onMove = (ev) => {{
                if (startX === null) startX = ev.clientX;
                if (!engaged) {{
                    // Moved before the hold completed → a scroll/flick, not a drag.
                    const dx = Math.abs(ev.clientX - startX);
                    const dy = Math.abs(ev.clientY - startY);
                    if (dx > MOVE_CANCEL || dy > MOVE_CANCEL) finish(-1);
                    return;
                }}
                card.style.transform = 'translateY(' + (ev.clientY - startY) + 'px)';
                // Target = last row whose original top is at/above the pointer
                // (clamps to the first/last row past the ends).
                const y = ev.clientY;
                let T = parseInt(rows[0].getAttribute('{index_attr}'));
                for (const r of rows) {{
                    if (y >= rects.get(r).top) T = parseInt(r.getAttribute('{index_attr}'));
                }}
                target = T;
                layout(T);
            }};
            const onUp = () => {{ finish(engaged && target !== draggedIndex ? target : -1); }};
            const onCancel = () => {{ finish(-1); }};
            window.addEventListener('pointermove', onMove);
            window.addEventListener('pointerup', onUp);
            window.addEventListener('pointercancel', onCancel);
            holdTimer = setTimeout(engage, HOLD_MS);
        }}
        "#
    );
    let mut eval = document::eval(&script);
    spawn(async move {
        if let Ok(idx) = eval.recv::<i32>().await
            && idx >= 0
            && idx as usize != dragged_index
        {
            on_drop(idx as usize);
        }
    });
}
