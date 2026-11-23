# AGENTS.md

## Publishing on hold

The user requested on 2026-11-11 that pending changes stay local. Do not push
commits/tags or publish a GitHub release unless the user explicitly requests
publishing again. Local edits, builds and checks remain authorized.

## Native Rust only

The user retired the Electron version on 2026-11-11. All widget UI runs in the
native Rust host. Do not restore Electron, React renderers, or WebViews.

All Node.js tooling/adapters and Python launchers are retired from runtime;
build and verify directly with Cargo and Python (`python scripts/check_safe.py`
and `cargo build --release --locked --manifest-path native-dock/Cargo.toml`, or
debug builds under `native-dock/target/debug/`). The user subsequently
authorized GitHub source and package publishing and requested maximum
portability on 2026-11-11. Native Rust portable ZIPs under `share/` are now in
scope; keep packaging builds under `native-dock/target/`. Do not restore
Electron or create installers.

- Keep `scripts/check_safe.py` passing; no PowerShell helper scripts.
- Do not enumerate local users/groups or inject keystrokes.
- Prefer source edits and restarting `kinetix.exe` for testing.
- Preserve `config/`, including legacy settings used for first-run migration.
- YouTube Music playback and CDP integration run natively in Rust under
  `widgets/music/native/backend/` (no Node.js, Electron, or WebView).
- App-wide logging writes to `logs/kinetix.log` (2 MB rotation) under the install/workspace
  root (`native-dock/src/logger.rs`), capturing app/widget lifecycle, panics,
  and SMTC fallback (`widgets/music/native/smtc.rs`).
- Always stop any running `kinetix.exe` instance before rebuilding `native-dock/target/release/kinetix.exe` (or `native-dock/target/debug/kinetix.exe`), and start it again after the build succeeds.

## 120Hz Every-Frame Scroll & Animation Standard

All scrollable views and animations across the native Rust app and widgets
(App Dock, Sticky Notes, Control Center, Control Center Manager, YouTube Music
including Coverflow transitions, Calendar, Clock, Widget Settings, and General
App Settings) must update on every single frame at 120Hz (~8.33 ms/frame):

- **VSync Frame Pacing (`pace_vsync`)**: Pair a 10 ms fallback `SetTimer` +
  `FrameClock` (`timeBeginPeriod(1)`) with post-`EndPaint` `DwmFlush()` +
  `PostMessageW(..., MOTION_FRAME, 0, 0)` (skipping `DwmFlush` if CPU+present
  exceeds 5.5 ms so a frame never oversleeps across a 120Hz VSync boundary).
- **Immediate First-Frame Wakeup**: Call `InvalidateRect` immediately when
  arming a scroll or animation timer so the first frame renders without waiting
  for a timer tick.
- **Zero Per-Frame GDI+ Context Creation on Cached Paths**: Keep `Batch::new`
  lazy (do not call `GdipCreateFromHDC` unless uncached GDI+ drawing occurs),
  draw scrollbars via software SDF (`Buffer::fill_round`), and use integer
  fixed-point math in `fade_list`, `fade_list_edges`, and
  `fade_horizontal_edges`.
- **Pixel-Aligned Orthogonal Axis & Fast Blitting**: During 1D scrolling, snap
  the orthogonal coordinate to device pixels (`fx == 0` for vertical scroll) so
  cached sprites/cards hit the `chunks_exact(4)` fast path in
  `Target::translate`, and suppress transient hover scale/glow trails on items
  passing under a stationary cursor while actively scrolling.

