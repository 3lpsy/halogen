# Halogen design reference

The current app, reconstructed as editable HTML contact sheets. iOS is the
primary target. Mobile web, desktop web and the desktop app follow as platform
variants. These are current-state mocks, not proposed layouts or a new theme.

Open [master.html](master.html) for the full deck. Each sheet has a PNG beside
it, stable frame labels and links to the source it follows. The separate
[capture gallery](captures.html) contains actual application screenshots.

## Layout

- `system/halogen.css` contains the shared mock styles for native iOS and Dioxus.
- `system/halogen.js` draws shared icons, status bars, docks and mini players.
- `system/01-components.html` records the current component shapes.
- `pages/start/` covers connection and the empty local library.
- `pages/library/` covers podcasts, episode browsing, Discover and OPML.
- `pages/listen/` covers Latest, queue, playlists, episode detail, the full
  player, downloads and history.
- `pages/settings/` covers More, settings, accounts, download config and cleanup.
- `pages/webui/` records mobile web, desktop web and desktop app differences.
- `LABELS.md` indexes the frames by stable label.
- `screenshots/` holds actual client captures. CI owns these inventories.
- `proposals/` remains separate for future unapproved changes.

## Updating and rendering

Edit a sheet and the shared system files, then run:

```sh
just design-build
just design-render pages/library/00-podcasts.html
```

`design-build` rebuilds the combined deck, frame index and capture gallery.
Run `./design/render.sh` without paths to render every sheet and system page.
Render one sheet per invocation when checking a subset, matching LiftFG's
workflow. Chromium and chromedriver are required; no package install is needed.

## Fidelity and evidence

The 2026-09-08 deck follows the Swift and Dioxus sources linked beneath each
sheet. The iOS shell uses the attached iOS 26 phone's conventional dock rather
than copying the historical simulator's floating dock. System typography,
SF Symbols and native controls are approximated with HTML and SVG. OS versions
can change their rendering without changing the app's navigation.

Artwork comes from existing podcast fixtures and a device capture. Sample
accounts use an example domain. Counts, playback positions and optional chapter
records illustrate existing data fields. Captions identify source-only states;
they do not claim a device or desktop test passed.

The current palettes intentionally differ: iOS uses native blue on black;
web and desktop use the existing gray Dioxus theme. The mocks preserve that
implementation difference. No product component changed for this deck.

Browser captures are from passing CI run 303 at `f37c5b2dd7` on 2026-09-08.
The native screenshot folders may contain older captures until their CI lanes
replace them. Read each capture inventory's provenance before treating it as
current evidence.
