# wgpu-gameui — contributor notes

Custom wgpu-based immediate-mode-ish game UI library. Widgets draw into a
`DrawList`, take `&InputState`/`&Theme`, and keep no internal frame state —
persistent state (scroll offset, drag ownership, focus) is **caller-owned** and
passed in by `&mut`. The public API surface is re-exported from `src/lib.rs`.

`TODO.md` is the working 1.0 backlog (P0/P1/P2). Cross items off as they land,
and append a short note describing the API that closed them.

## When you add or change a widget

A widget isn't done until **all** of these are true — treat it as the checklist:

1. **Module + export.** New file under `src/widgets/`, wired in
   `src/widgets/mod.rs` (`mod foo;` + `pub use foo::Foo;`). Crate-root
   re-export via `pub use widgets::*` makes it reachable as `wgpu_gameui::Foo`.
2. **Unit tests.** Cover the geometry/state logic with headless tests against a
   `DrawList` (inspect `list.icons` / `list.vertices`) — no GPU needed. Respect
   `InputState::mouse_consumed` for anything clickable (layer capture).
3. **Widget gallery.** Add a row to `tests/widget_gallery.rs` so the widget is
   visible in the rendered PNG. This is mandatory, not optional — the gallery is
   how we eyeball every widget at once and catch layout/visual regressions.
   Render and *look at the result*:
   ```
   DISPLAY=:0 cargo test --test widget_gallery -- --ignored --nocapture
   ```
   Writes `test_output/widget_gallery.png`. Rendering it has already caught real
   bugs (e.g. a too-large default padding that shrank an icon to a speck) that
   the unit tests passed straight through — so don't skip the eyeball pass.
4. **TODO.md.** Check off the item and note the resulting API.

## Conventions

- **Caller-owned state.** Don't stash mutable widget state in globals or
  thread-locals; take it by `&mut` (see `ScrollState`, `DragCapture`). The one
  façade that holds state is `UiContext` (the Teardown-verb layer).
- **Dual sprite sources.** Image-bearing widgets accept both a pre-resolved
  `SpriteId` (supports tint + UV crop) and a string key resolved by name at
  render time (no tint/crop). See `Image` / `Checkbox` / `ImageButton`.
- **Theme-relative sizing.** Derive paddings/sizes from `Theme` fields rather
  than hard-coding pixels, so DPI scaling and re-theming work.
- **Rect-native draw entry points.** Prefer `draw(rect, ...)` taking a
  layout-computed `Rect`; `Rect::contains` is edge-exclusive.

## Build / test

```
cargo build --all-targets      # must be warning-clean
cargo test --lib               # all widget/layout/text unit tests
```
