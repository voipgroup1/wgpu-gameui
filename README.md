# wgpu-gameui

A custom, wgpu-based immediate-mode game UI library. Originally extracted from a
city builder game and built to replace egui in contexts where UI *aesthetics*
matter — polished chrome, MSDF text with outlines/shadows/glow, nine-slice
framing, per-subtree styling, and a Teardown-style immediate-mode verb API.

- **Render-only.** No windowing, audio, or device I/O — the app owns the event
  loop and fills a plain `InputState` struct. Renders through a single
  `UiRenderer::render` call into a wgpu `TextureView`.
- **Immediate-mode.** Build a `DrawList` per frame from widget calls; nothing
  is retained across frames except the caller-owned state structs (`UiState`,
  `ScrollState`, `FocusState`, `DragCapture`, …).
- **MSDF text.** Glyphs are rendered through a custom multi-channel signed
  distance field atlas (via `fdsm`), so text supports outlines, shadows, and
  glow at any zoom without re-rasterizing. Shaping is cosmic-text.
- **Dual API.** Draw raw widgets against a `DrawContext` for full control, or
  use the `UiContext` / `Frame` façade for auto-advancing, stateful verbs
  (`text_button`, `slider`, `text_input`, …) — the Teardown port target.

Dual-licensed MIT OR Apache-2.0.

---

## Quickstart

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
wgpu-gameui = "0.1"
```

The default features bundle Noto Sans (so the UI renders identically everywhere
without system fonts) and the Phosphor icon font. Disable them to slim the
binary:

```toml
wgpu-gameui = { version = "0.1", default-features = false }
```

### Minimal render loop

The library is windowing-agnostic. You bring the event loop (e.g. `winit`) and
wgpu surface; the UI side is three steps per frame:

```rust,ignore
use wgpu_gameui::{DrawList, InputState, Theme, UiRenderer, KeyboardNav, UiState, Frame};

// --- 1. Setup (once) -------------------------------------------------
let font_system = wgpu_gameui::shared_font_system();
let mut ui_renderer = UiRenderer::new(&device, &queue, surface_format, font_system);
let theme = Theme::default();

// --- 2. Per-frame state (owned by the app, persists across frames) ---
let mut ui_state = UiState::new();
let mut input = InputState::default();

// --- 3. Frame loop ---------------------------------------------------
// Fill `input` from your window events (mouse, keyboard, wheel, text), then:
let mut list = DrawList::with_font_system(wgpu_gameui::shared_font_system());

Frame::new(&mut ui_state, &mut input, &theme, &KeyboardNav)
    .dt(0.016) // frame delta for hover/press easing
    .run(&mut list, |ui| {
        if ui.text_button("Play", Some(120.0), None) {
            // start the game
        }
        let mut name = String::from("Player");
        ui.text_input(0, &mut name, "name…", Some(200.0));
    });

// Render the DrawList:
let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
ui_renderer.render(&device, &queue, &mut encoder, &view, (width, height), scale, &list);
queue.submit(Some(encoder.finish()));

// Clear per-frame input edges after all surfaces/layers are done:
input.end_frame();
```

Run the full interactive example (window + mouse + keyboard + dropdowns +
modals + scroll views + text inputs):

```
cargo run --example hello_ui
```

---

## Widget gallery

| Widget | Façade verb (`UiContext`) | Raw widget |
|---|---|---|
| Button | `text_button(label, w, h) -> bool` | `Button::draw(rect, ctx)` |
| Checkbox | `checkbox(label, checked) -> bool` | `Checkbox::draw(checked, label, rect, ctx)` |
| Slider | `slider(id, value, min, max, w) -> f32` | `Slider::draw(value, id, capture, rect, ctx)` |
| Radio group | `radio_group(options, selected) -> usize` | `RadioGroup::draw(selected, rect, ctx)` |
| Text input | `text_input(id, buf, placeholder, w) -> bool` | `TextInput::draw(id, ctx)` |
| Password input | `password_input(id, buf, placeholder, w)` | `TextInput::password()` |
| Text area | `text_area(id, buf, placeholder, w, rows)` | `TextInput::with_multiline(true)` |
| Number input | `number_input(id, val, min, max, step, dec, w)` | `NumberInput::draw(val, id, ti, rect, ctx)` |
| Dropdown | `dropdown(id, options, selected, w)` | `Dropdown::draw(ctx)` + `DropdownState` |
| Tree | `tree_node` / `tree_leaf` / `tree_pop` | `TreeNode::draw(rect, ctx)` |
| Tabs | `tabs(labels, active) -> Option<usize>` | `Tabs::draw(rect, list, style, input)` |
| Scroll view | `scroll_begin(w, h) -> Rect` / `scroll_end()` | `ScrollView::draw(state, list, style, input, closure)` |
| Enabled/disabled subtree | `enabled_scope(enabled, \|ui\|)` / `disabled_scope(\|ui\|)` | *(scope verb — gray-tint + input-disable a block)* |
| List | *(raw widget)* | `List::draw(rect, count, state, list, style, input, closure)` |
| Table | *(raw widget)* | `Table::draw(rect, rows, scroll, list, style, input)` |
| Panel | `panel(w, h)` | `Panel::draw_at(rect, list, style)` |
| Group | `group_begin(title, w, h) -> Rect` | `Group::draw(rect, list, style) -> Rect` |
| Separator | `separator()` | `Separator::draw(rect, list, style)` |
| Progress bar | `progress_bar(value, w)` | `ProgressBar::draw(rect, list, style)` |
| Banner | `banner(severity, message, w)` | `Banner::draw(rect, list, style)` |
| Color picker | `color_picker(id, hsva, w)` | `ColorPicker::draw(hsva, id, capture, rect, ctx)` |
| Drag handle | `drag_handle(id, w, h)` | `DragHandle::draw(rect, id, capture, ctx)` |
| Image button | `image_button_key(key, w, h)` | `ImageButton::draw(rect, list, style, input)` |
| Image | `image_box(sprite, w, h)` | `Image::draw(rect, list)` |
| Icon | *(draw primitive)* | `DrawList::icon` / `Icon` widget |
| Hit zone | `hit_zone(w, h)` / `hit_zone_at(rect)` | `HitZone::test(rect, input)` |
| Toast | *(state on `UiState::toasts`)* | `ToastStack::push` / `tick` / `draw` |
| Tooltip | *(state on `UiState::tooltips`)* | `TooltipLayer::hover_zone` / `tick` / `draw` |

> **List and Table** stay raw widgets (no façade verb) because their
> closure-based row/cell APIs don't fit the simple auto-advance verb model.

A headless render of the full widget set is checked into the test suite:

```
cargo test --test widget_gallery -- --ignored --nocapture
# writes test_output/widget_gallery.png
```

---

## Architecture

### Three layers of API

```
┌──────────────────────────────────────────────────────┐
│  Frame::run (closure-scoped begin/end_frame bracket) │  ← easiest
├──────────────────────────────────────────────────────┤
│  UiContext verbs (text_button, slider, text_input…)  │  ← façade
├──────────────────────────────────────────────────────┤
│  Raw widgets (Button::draw, Slider::draw, …)         │  ← full control
│  + DrawList primitives (quad, rounded_rect, line, …)  │
└──────────────────────────────────────────────────────┘
```

- **`DrawList`** is the pure data layer: a CPU-side command list of quads,
  rounded rects, lines, circles, icons, nine-slices, and text blocks. It owns
  the transform stack, tint stack, and clip stack. No GPU state.
- **`DrawContext`** bundles a `&mut DrawList` with `&mut FocusState`,
  `&Theme`, `&InputState`, and screen dimensions, plus optional seams for
  animation (`with_animations`) and cursor requests (`with_cursor`). Raw
  widgets take this.
- **`UiContext`** is a thin borrow over a `DrawList` (in interactive mode,
  also `&InputState` + `&mut UiState` + `&Theme`) that adds Teardown-style
  verbs: `push`/`pop`, `translate`/`rotate`/`scale`, `align`/`center`,
  `color`/`color_filter`, `place_rect`, and the auto-advancing widget verbs.
- **`Frame`** is the closure-scoped entry point: it runs `begin_frame` /
  `end_frame` around your build closure so the pair can't be forgotten or
  mis-ordered, and `UiContext` is dropped (firing push/pop balance checks)
  before `end_frame`.

### Rendering pipeline

`UiRenderer` owns the wgpu pipelines, a dynamic sprite atlas, a nine-slice
metadata table, and the MSDF glyph atlas. `render(&DrawList)` tessellates and
encodes four sub-passes in order:

```
nine-slices → colored quads → icons → MSDF text
```

`render_layers(&LayerStack)` does the same for each layer in z-order, so a
popup's quads correctly overlap a base layer's text. The renderer never samples
its own framebuffer; `blur_backdrop` takes an app-provided scene texture for
frosted-glass effects.

### Image & atlas lifecycle

Images enter the sprite atlas three ways: `load_image_file` / `load_image_bytes`
(decode PNG/JPEG), `load_image_rgba8` (already-decoded pixels — skips the decode
round-trip, for apps that hold raw buffers like rendered notification icons), and
the out-of-band `load_sprite_rgba8`. The first three are keyed and cached, so
`has_image` / `image_size` / `unload_image` see them. The atlas grows on demand
(1024 → 2048 → 4096) and `SpriteId`s are stable indices that never shift.

`unload_image` frees the slot immediately (its pixels are reclaimed and the slot
recycled); shelf *fragmentation* left by churn is reclaimed by `compact_atlas`,
which a long-running app should call periodically (gate on `atlas_size()`
approaching a threshold) to keep the texture from climbing toward its 4096² cap.

### Caller-owned state

The library is immediate-mode, but interaction state persists across frames in
caller-owned structs. Construct them once, thread `&mut` into the relevant
widgets each frame:

| State struct | Owns | Used by |
|---|---|---|
| `UiState` | focus, drag capture, dropdowns, scroll, tree, animations, text inputs, toasts, tooltips | `UiContext` verbs, `Frame::run` |
| `ScrollState` | scroll offset + content extent | `ScrollView`, `List`, `Table` |
| `ListState` | scroll + selection set + keyboard cursor | `List` |
| `FocusState` | single focus owner + Tab ring | `TextInput`, `Button`, `Checkbox`, `Slider` |
| `DragCapture` | single drag owner (arbitration) | `Slider`, `ScrollView`, `DragHandle`, `ColorPicker` |
| `DropdownState` | which dropdown is open + geometry | `Dropdown` |
| `TreeState` | expanded set + selection + nav ring | `TreeNode` |
| `AnimationState` | in-flight color/scalar transitions | animated widgets via `DrawContext::with_animations` |
| `DragTracker` | press origin + click-vs-drag latch | writes `input.is_dragging` / `drag_delta` |
| `ClickTracker` | double-click + hold detection | writes `input.mouse_double_clicked` / `mouse_held` |
| `CursorState` | per-frame cursor icon accumulator | widgets request via `DrawContext::request_cursor` |

### Layout

A separate flexbox-style layout system (`layout` module) computes `Rect`s from
a tree of `VStack` / `HStack` / `Flow` / `Positioned` nodes. It does not touch
`DrawList` — you call `layout_screen(w, h)` once, then draw widgets at the
resulting rects. Supports `Fill`/`Fixed`/`Percent`/`Fit` sizing, weighted
flex-grow, `CrossAlign`, `MainAlign` (justify-content), wrap/flow, min/max
constraints, and stable node IDs for order-independent lookup.

### Theming & styling

`Theme` is a flat struct of colors + font + spacing. Every widget resolves
style through a `StyleResolver` (precedence: `StyleOverlay` → `Theme`), so a
subtree can be restyled without cloning the theme. `UiContext::set_style_color`
/ `set_style_scalar` push scoped overrides. Custom keys via
`StyleKey::custom(name)` + `Theme::register_style`. Hover/press color
transitions via `AnimationState` (eased, with a `0.0`-duration fast path that
is byte-identical to the instant path).

### Input & focus

The app fills an `InputState` struct (mouse position/buttons, scroll delta,
keyboard edges, text input, IME preedit, and a device-agnostic `NavInput` for
keyboard/gamepad navigation). The library never reads devices. Layer-aware
input dispatch (`LayerStack::input_for_base` / `input_for_layer`) sets
`mouse_consumed` so lower layers don't fire through popups/modals. Tab focus
cycles through registered `FocusId`s, scoped to the active layer.

---

## Features

| Feature | Default | Description |
|---|---|---|
| `bundled-font` | ✅ | Embed Noto Sans (regular/bold/italic) as the default sans-serif. Drop ~1.5 MB if you supply your own fonts. |
| `phosphor-icons` | ✅ | Embed the Phosphor (MIT) icon font and expose the `PhosphorIcon` enum + `Icon` widget. Drop ~0.5 MB if unused. |
| `tracy` | ❌ | Emit `tracing` spans around the render path for Tracy profiling. |

---

## Testing & benchmarks

```bash
# Unit tests (696 tests, headless, no GPU)
cargo test --lib

# Widget gallery (headless GPU render → PNG)
cargo test --test widget_gallery -- --ignored --nocapture

# Benchmarks (CPU-only groups need no GPU; render groups do)
DISPLAY=:0 cargo bench --bench ui_stress
```

Benchmark groups: `drawlist_build`, `frame_render`, `render_text_only`,
`nine_slice`, `icons`, `primitives_build`, `primitives_render`, `layout_resolve`,
`text_shape`, `interactive_widgets`, `text_input_edit`, `scroll_view`,
`list_virtual`, `table`, `ui_context_frame`, `animation`.

---

## License

MIT OR Apache-2.0, at your option.
