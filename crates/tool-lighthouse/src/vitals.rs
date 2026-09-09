//! Install buffered PerformanceObservers once per document, interact, then drain/reset window.__lh. Buffered observers
//! recover navigation events even when installed later. Record layout-shift nodes and before/after rectangles so
//! reports identify the moved element.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thirtyfour::WebDriver;

/// Long-task duration (ms) above which time counts as "blocking" — the same 50ms
/// threshold Lighthouse uses to derive Total Blocking Time.
const BLOCKING_THRESHOLD_MS: f64 = 50.0;

/// Steps whose CLS is at least this get a culprit breakdown in the summary.
const CULPRIT_CLS_THRESHOLD: f64 = 0.05;

/// Installs the observers (idempotent — guarded on `window.__lh`). Layout-shift → summed CLS (ignoring shifts
/// within 500ms of input, like CLS proper) PLUS a per-shift record of its source nodes + rects; long tasks →
/// durations; `event` timing → max duration (an INP proxy); LCP + FCP from the paint/LCP entries. All
/// `buffered` so a post-navigation install still sees the load.
pub const COLLECTOR: &str = r#"
if (!window.__lh) {
    window.__lh = { cls: 0, lt: [], lcp: 0, inp: 0, fcp: 0, shifts: [], rowlog: [] };
    // Frame-rate logger for the episode list: records every change to the row
    // count, the first row's id, OR the first row's node identity (a remount where
    // the count is unchanged but the DOM nodes were replaced). Catches sub-300ms
    // teardown/rebuild that coarse polling misses.
    let __lastSig = null, __lastNode = null, __stamp = 0;
    const __sample = () => {
        try {
            const rows = document.querySelectorAll('#episode-scroll [id^="ep-row-"]');
            const sk = document.querySelectorAll('#episode-scroll .skeleton').length;
            const first = rows[0] || null;
            const remount = first !== __lastNode;
            __lastNode = first;
            // Node-survival: stamp each row the first time it's seen. On a change,
            // count how many current rows carry an OLD stamp (survived) vs are NEW.
            // survived≈old-count → keyed diff (nodes kept); survived=0 → full teardown.
            let survived = 0, fresh = 0;
            for (const r of rows) {
                if (r.__st === undefined) { r.__st = ++__stamp; fresh++; }
                else survived++;
            }
            const sig = rows.length + '|' + sk + '|' + (first ? first.id : '-');
            if (sig !== __lastSig || (remount && rows.length)) {
                if (window.__lh.rowlog.length < 200) {
                    window.__lh.rowlog.push({ t: Math.round(performance.now()), n: rows.length, sk, top: first ? first.id.replace('ep-row-', '') : '-', r: remount ? 1 : 0, kept: survived, new: fresh });
                }
                __lastSig = sig;
            }
        } catch (e) {}
        requestAnimationFrame(__sample);
    };
    requestAnimationFrame(__sample);
    const rect = (r) => r ? { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) } : null;
    const describe = (n) => {
        if (!n || n.nodeType !== 1) return (n && n.nodeName) ? n.nodeName.toLowerCase() : '(anonymous)';
        let s = n.tagName.toLowerCase();
        if (n.id) s += '#' + n.id;
        if (n.classList && n.classList.length) s += '.' + Array.from(n.classList).slice(0, 3).join('.');
        const dio = n.getAttribute && n.getAttribute('data-dioxus-id');
        if (dio) s += '[dx=' + dio + ']';
        const t = (n.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 48);
        if (t) s += ' "' + t + '"';
        return s;
    };
    const obs = (type, cb) => { try { new PerformanceObserver(cb).observe({ type, buffered: true }); } catch (e) {} };
    obs('layout-shift', (l) => {
        for (const e of l.getEntries()) {
            if (e.hadRecentInput) continue;
            window.__lh.cls += e.value;
            const sources = (e.sources || []).map((s) => ({ node: describe(s.node), from: rect(s.previousRect), to: rect(s.currentRect) }));
            window.__lh.shifts.push({ value: e.value, sources });
        }
    });
    obs('largest-contentful-paint', (l) => { const es = l.getEntries(); const last = es[es.length - 1]; if (last) window.__lh.lcp = last.startTime; });
    obs('longtask', (l) => { for (const e of l.getEntries()) window.__lh.lt.push(e.duration); });
    obs('event', (l) => { for (const e of l.getEntries()) if (e.duration > window.__lh.inp) window.__lh.inp = e.duration; });
    obs('paint', (l) => { for (const e of l.getEntries()) if (e.name === 'first-contentful-paint') window.__lh.fcp = e.startTime; });
}
return true;
"#;

/// Returns the accumulated metrics (largest shifts first) and resets the window.
pub const DRAIN: &str = r#"
const d = window.__lh || { cls: 0, lt: [], lcp: 0, inp: 0, fcp: 0, shifts: [], rowlog: [] };
const shifts = (d.shifts || []).slice().sort((a, b) => b.value - a.value).slice(0, 5);
const out = { cls: d.cls, longtasks: d.lt.slice(), lcp: d.lcp, inp: d.inp, fcp: d.fcp, shifts, rowlog: (d.rowlog || []).slice() };
if (window.__lh) { window.__lh.cls = 0; window.__lh.lt = []; window.__lh.inp = 0; window.__lh.lcp = 0; window.__lh.fcp = 0; window.__lh.shifts = []; window.__lh.rowlog = []; }
return out;
"#;

/// Element rect: x/y drive the movement delta; `h` (height) tells us what a
/// resized element actually rendered at — the number to set `contain-intrinsic-size`
/// to. `w` is sent by the page but unused, so it's not deserialized.
#[derive(Deserialize, Clone, Copy, Default)]
struct RawRect {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    h: f64,
}

#[derive(Deserialize, Default)]
struct RawSource {
    #[serde(default)]
    node: String,
    #[serde(default)]
    from: Option<RawRect>,
    #[serde(default)]
    to: Option<RawRect>,
}

#[derive(Deserialize, Default)]
struct RawShift {
    #[serde(default)]
    value: f64,
    #[serde(default)]
    sources: Vec<RawSource>,
}

/// Raw drain payload from the page.
#[derive(Deserialize, Default)]
struct RawVitals {
    #[serde(default)]
    cls: f64,
    #[serde(default)]
    longtasks: Vec<f64>,
    #[serde(default)]
    lcp: f64,
    #[serde(default)]
    inp: f64,
    #[serde(default)]
    fcp: f64,
    #[serde(default)]
    shifts: Vec<RawShift>,
    #[serde(default)]
    rowlog: Vec<RowSample>,
}

/// One frame-logger sample of the episode list: `t`ime (ms), row `n`umber,
/// `sk`eleton count, `top` row id, and `r`emount flag (first node identity changed).
#[derive(Deserialize, Default)]
struct RowSample {
    #[serde(default)]
    t: f64,
    #[serde(default)]
    n: usize,
    #[serde(default)]
    sk: usize,
    #[serde(default)]
    top: String,
    #[serde(default)]
    r: u8,
    #[serde(default)]
    kept: usize,
    #[serde(default)]
    new: usize,
}

/// One moving element within a layout shift: which node, and its height before→
/// after (`178→0` = removed/unmounted; `0→178` = added/mounted; `178→178` = moved).
#[derive(Serialize, Clone)]
pub struct SourceInfo {
    pub node: String,
    pub h_from: f64,
    pub h_to: f64,
    pub dy: f64,
}

/// One layout-shift's culprit: how much it scored, plus EVERY node it moved. A
/// removed node paired with an added node (`178→0` + `0→178`) is a swap/remount;
/// many nodes is a whole-region reflow.
#[derive(Serialize, Clone)]
pub struct ShiftInfo {
    pub value: f64,
    pub node: String,
    pub dx: f64,
    pub dy: f64,
    pub h_from: f64,
    pub h_to: f64,
    pub source_count: usize,
    /// Every source node of this shift (largest-area first as reported by the
    /// browser), so a swap (removed + added) is visible, not just the top mover.
    pub sources: Vec<SourceInfo>,
}

/// One measured step's vitals (derived from a [`RawVitals`]).
#[derive(Serialize, Clone)]
pub struct StepVitals {
    pub name: String,
    /// Cumulative Layout Shift accumulated during the step.
    pub cls: f64,
    /// Σ over long tasks of `max(0, dur - 50ms)` — a Total-Blocking-Time proxy.
    pub blocking_ms: f64,
    pub longtask_count: usize,
    pub longest_task_ms: f64,
    /// Worst interaction latency in the window (INP proxy).
    pub inp_ms: f64,
    pub lcp_ms: f64,
    pub fcp_ms: f64,
    /// The largest layout shifts in the step, each with its culprit node.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shifts: Vec<ShiftInfo>,
}

/// Reduce a raw shift to its most-informative source (one with real rects), and
/// compute how far that element moved.
fn shift_info(raw: &RawShift) -> ShiftInfo {
    let src = raw
        .sources
        .iter()
        .find(|s| s.from.is_some() && s.to.is_some())
        .or_else(|| raw.sources.first());
    let (node, dx, dy, h_from, h_to) = match src {
        Some(s) => {
            let (dx, dy) = match (s.from, s.to) {
                (Some(f), Some(t)) => (t.x - f.x, t.y - f.y),
                _ => (0.0, 0.0),
            };
            let node = if s.node.is_empty() {
                "(anonymous)".to_string()
            } else {
                s.node.clone()
            };
            (
                node,
                dx,
                dy,
                s.from.map(|r| r.h).unwrap_or(0.0),
                s.to.map(|r| r.h).unwrap_or(0.0),
            )
        }
        None => ("(no source)".to_string(), 0.0, 0.0, 0.0, 0.0),
    };
    let sources = raw
        .sources
        .iter()
        .map(|s| {
            let dy = match (s.from, s.to) {
                (Some(f), Some(t)) => t.y - f.y,
                _ => 0.0,
            };
            SourceInfo {
                node: if s.node.is_empty() {
                    "(anonymous)".to_string()
                } else {
                    s.node.clone()
                },
                h_from: s.from.map(|r| r.h).unwrap_or(0.0),
                h_to: s.to.map(|r| r.h).unwrap_or(0.0),
                dy,
            }
        })
        .collect();
    ShiftInfo {
        value: raw.value,
        node,
        dx,
        dy,
        h_from,
        h_to,
        source_count: raw.sources.len(),
        sources,
    }
}

/// Install the collector into the current document (call once per page load).
pub async fn install(driver: &WebDriver) -> Result<()> {
    driver
        .execute(COLLECTOR, Vec::new())
        .await
        .context("install PerformanceObserver collector")?;
    Ok(())
}

/// Drain + reset the window, returning this step's vitals under `name`. `is_load` marks a page-load step:
/// LCP/FCP are load metrics, so on interaction steps (scroll/search/click) they're zeroed — otherwise a lazy
/// image becoming a late LCP candidate mid-scroll reports a bogus multi-second "LCP".
pub async fn drain(driver: &WebDriver, name: &str, is_load: bool) -> Result<StepVitals> {
    let ret = driver
        .execute(DRAIN, Vec::new())
        .await
        .context("drain PerformanceObserver buffer")?;
    let raw: RawVitals =
        serde_json::from_value(ret.json().clone()).context("decode drained vitals")?;
    // `+ 0.0` normalizes a possible `-0.0` (so it prints as "0", not "-0").
    let blocking_ms: f64 = raw
        .longtasks
        .iter()
        .map(|d| (d - BLOCKING_THRESHOLD_MS).max(0.0))
        .sum::<f64>()
        + 0.0;
    let longest_task_ms = raw.longtasks.iter().copied().fold(0.0_f64, f64::max);
    if is_load && !raw.rowlog.is_empty() {
        let seq: Vec<String> = raw
            .rowlog
            .iter()
            .map(|s| {
                let sk = if s.sk > 0 {
                    format!("/{}sk", s.sk)
                } else {
                    String::new()
                };
                let rm = if s.r == 1 { "⟲" } else { "" };
                let top = if s.top == "-" {
                    String::new()
                } else {
                    format!(" top={}", s.top)
                };
                // Only annotate survivorship on real row frames (not skeleton-only).
                let churn = if s.n > 0 {
                    format!(" (kept {} new {})", s.kept, s.new)
                } else {
                    String::new()
                };
                format!("{}ms:{}{}row{}{}{}", s.t as u64, rm, s.n, sk, top, churn)
            })
            .collect();
        eprintln!("   [{name}] rowlog: {}", seq.join("  →  "));
    }
    let shifts: Vec<ShiftInfo> = raw.shifts.iter().take(3).map(shift_info).collect();
    Ok(StepVitals {
        name: name.to_string(),
        cls: raw.cls,
        blocking_ms,
        longtask_count: raw.longtasks.len(),
        longest_task_ms,
        inp_ms: raw.inp,
        lcp_ms: if is_load { raw.lcp } else { 0.0 },
        fcp_ms: if is_load { raw.fcp } else { 0.0 },
        shifts,
    })
}

/// Reset the window without recording (establish a clean baseline before an
/// interaction).
pub async fn baseline(driver: &WebDriver) -> Result<()> {
    driver
        .execute(DRAIN, Vec::new())
        .await
        .context("reset vitals baseline")?;
    Ok(())
}

/// Print a one-row-per-step table + a layout-shift culprit breakdown to stdout.
pub fn print_summary(base_url: &str, steps: &[StepVitals]) {
    println!("\n── Web-Vitals walkthrough: {base_url} ──");
    println!(
        "{:<26} {:>6} {:>10} {:>6} {:>8} {:>8} {:>8}",
        "Step", "CLS", "Block ms", "LTasks", "INP ms", "LCP ms", "FCP ms"
    );
    println!("{}", "-".repeat(80));
    for s in steps {
        println!(
            "{:<26} {:>6.3} {:>10.0} {:>6} {:>8.0} {:>8.0} {:>8.0}",
            truncate(&s.name, 26),
            s.cls,
            s.blocking_ms,
            s.longtask_count,
            s.inp_ms,
            s.lcp_ms,
            s.fcp_ms,
        );
    }
    let worst_cls = steps.iter().map(|s| s.cls).fold(0.0_f64, f64::max);
    let total_block: f64 = steps.iter().map(|s| s.blocking_ms).sum();
    println!("{}", "-".repeat(80));
    println!("worst step CLS: {worst_cls:.3}   total blocking time: {total_block:.0} ms");

    let mut header = false;
    for s in steps {
        if s.cls < CULPRIT_CLS_THRESHOLD || s.shifts.is_empty() {
            continue;
        }
        if !header {
            println!("\nLayout-shift culprits (CLS ≥ {CULPRIT_CLS_THRESHOLD}):");
            header = true;
        }
        println!("  {} — CLS {:.3}", s.name, s.cls);
        for sh in &s.shifts {
            println!(
                "      shift {:.3}  ({} node(s)):",
                sh.value, sh.source_count
            );
            for src in &sh.sources {
                println!(
                    "        h {:.0}→{:.0}px  Δy {:+.0}px  {}",
                    src.h_from, src.h_to, src.dy, src.node
                );
            }
        }
    }
}

/// Write `report.json` (machine-readable) + `report.md` (human-readable) to `out`.
pub fn write_reports(out: &Path, base_url: &str, build: &str, steps: &[StepVitals]) -> Result<()> {
    let json = serde_json::json!({
        "target": base_url,
        "build": build,
        "note": "PerformanceObserver Web Vitals (no synthetic throttling unless --cpu/--throttle-network); not a Lighthouse score",
        "steps": steps,
    });
    std::fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(&json)?,
    )
    .context("write report.json")?;

    let mut md = String::new();
    md.push_str(&format!(
        "# Web-Vitals walkthrough\n\n**Target:** {base_url}  \n**Build:** `{}`\n\n",
        if build.is_empty() { "(unknown)" } else { build }
    ));
    md.push_str(
        "_PerformanceObserver metrics from a real Chrome run. Absolute values depend on \
         `--cpu`/`--throttle-network`; use for catching regressions across runs._\n\n",
    );
    md.push_str(
        "| Step | CLS | Blocking ms | Long tasks | Longest ms | INP ms | LCP ms | FCP ms |\n",
    );
    md.push_str("|---|---:|---:|---:|---:|---:|---:|---:|\n");
    for s in steps {
        md.push_str(&format!(
            "| {} | {:.3} | {:.0} | {} | {:.0} | {:.0} | {:.0} | {:.0} |\n",
            s.name,
            s.cls,
            s.blocking_ms,
            s.longtask_count,
            s.longest_task_ms,
            s.inp_ms,
            s.lcp_ms,
            s.fcp_ms,
        ));
    }

    let culprit_steps: Vec<&StepVitals> = steps
        .iter()
        .filter(|s| s.cls >= CULPRIT_CLS_THRESHOLD && !s.shifts.is_empty())
        .collect();
    if !culprit_steps.is_empty() {
        md.push_str(&format!(
            "\n## Layout-shift culprits (CLS ≥ {CULPRIT_CLS_THRESHOLD})\n\n"
        ));
        for s in culprit_steps {
            md.push_str(&format!("### {} — CLS {:.3}\n\n", s.name, s.cls));
            for sh in &s.shifts {
                md.push_str(&format!(
                    "- shift **{:.3}** ({} node(s)):\n",
                    sh.value, sh.source_count
                ));
                for src in &sh.sources {
                    md.push_str(&format!(
                        "    - h {:.0}→{:.0}px, Δy {:+.0}px — `{}`\n",
                        src.h_from, src.h_to, src.dy, src.node
                    ));
                }
            }
            md.push('\n');
        }
    }

    std::fs::write(out.join("report.md"), md).context("write report.md")?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
