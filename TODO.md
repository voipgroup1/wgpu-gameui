# wgpu-gameui TODO

Consolidated from two independent audits (Claude + Codex) on 2026-04-26 of the
~2,630 LOC source tree just extracted from citybuilder. Both audits agreed on
the same major gaps. Items grouped by category and tagged with priority:

- **P0** — blocks 1.0 / blocks Teardown-API port
- **P1** — important, but the lib is usable without it
- **P2** — nice-to-have

Use this as the working backlog for the package. Cross items off as PRs land.

---

## Architecture / Core Plumbing

- [x] **P0 — Public `UiRenderer`/`Backend`** that owns the wgpu pipeline, sampler,
      atlas, and consumes a `DrawList` per frame. (`src/render/ui_renderer.rs`)
- [x] **P0 — Texture atlas / sprite registry.** Dynamic shelf packer with grow
      (`src/render/atlas.rs`), `load_sprite_rgba8` + `register_nine_slice` on
      `UiRenderer`. `IconDraw`/`NineSliceDraw` now carry pre-resolved
      `SpriteId`/`NineSliceId` with name fallback.
- [x] **P0 — Matrix / transform stack** (`UiPush`/`UiPop`/`UiTranslate`/`UiAlign`/
      `UiCenter`/`UiRotate`/`UiScale`). 2x3 affine stack lives on `DrawList`
      so existing widgets that take absolute `Rect`s pick it up transparently;
      `UiContext` (`src/ui_context.rs`) is the Teardown-verb façade.
- [x] **P0 — Color / tint stack** (`UiColor`/`UiColorFilter`) with sub-tree alpha
      multiplier. Lives alongside the transform stack on `DrawList`; primitive
      methods multiply input color by current tint at push time.
- [x] **P0 — Clip / scissor stack.** `push_clip(rect)`/`pop_clip()` with draw
      commands grouped per clip stack. `Table::draw_cell` text is currently
      *not actually clipped* by `content_rect`.
- [x] **P1 — Unify widget API around `DrawContext` + `Rect`.** `Dropdown::draw`
      and `Button::draw`/`draw_at`/`draw_nine_slice` now take `&mut DrawContext`
      instead of individual `(&mut DrawList, &Theme, &InputState)` params.
      `DrawContext::register_focus(id)` auto-scopes to the active layer.
      Checkbox, Slider, and TextInput now take `&mut DrawContext` too. Remaining
      non-interactive widgets (Tabs, ProgressBar, ScrollView, Table, Tooltip,
      ImageButton) still take individual params — deferred to follow-up passes.
- [x] **P1 — Replace `String` keys in draw commands with interned `IconId`/
      `SpriteId`/`u32` handles** produced by the atlas. (Both still accept
      string-keyed helpers for ergonomics; `icon_sprite`/`nine_slice_id` are the
      allocation-free path.)
- [x] **P1 — Don't `unwrap()` glyphon errors in `TextRenderer::prepare`/
      `render`** (`src/text.rs:103,130`). ~~Bubble as a typed `UiError`.~~
      Obsoleted by the MSDF rewrite (Phase 1-3): glyphon's fallible GPU
      `prepare`/`render` stage was replaced by the custom `MsdfGlyphAtlas` path.
      There is no `fn prepare` anymore and `render` is infallible — a crate-wide
      grep finds zero uses of glyphon's GPU error API. The only remaining
      `expect`s in `text.rs` are `FontSystem` lock-poison guards (another thread
      panicked holding the lock = unrecoverable; propagating the panic is the
      correct idiom, not a recoverable `UiError`).
- [x] **P1 — Cache glyphon `Buffer`s by content+size hash or pool them.**
      Done better than pooling buffers: `TextRenderer::build_vertices` caches the
      *shaped glyph layout* (relative positions) keyed by
      `(content, font_size, line_height, max_width, family, align, ellipsize,
      weight, style)`. A hit skips `Buffer::new`/`set_text`/`shape_until_scroll`
      and takes no `FontSystem` lock (the MSDF atlas never evicts, so cached
      glyphs are always present). Working-set eviction past `SHAPE_CACHE_MAX`
      (8192); `clear_shape_cache()` for font hot-loads. (`src/text.rs`)

---

## Draw Primitives

- [x] **P0 — Rounded rectangles.** `theme.border_radius` exists but is never
      used. Teardown's `UiRoundedRect` is widely used. Tessellate or SDF.
- [x] **P0 — Lines / strokes** (`line(p0, p1, thickness, color)`) with
      thickness/joins/caps/AA. Teardown's `DrawLine` is top-30. Also needed
      for slider tick marks, debug overlays.
- [x] **P1 — Circles / arcs / ellipses** (`DrawList::circle`, `circle_outline`, `stroked_arc`; `UiContext::circle`, `circle_outline`).
- [x] **P1 — Textured quad with explicit UV rect.** Atlas `AtlasRegion::uv()`
      drives icon UVs, with optional tint per draw.
- [x] **P1 — Nine-slice border metadata.** `register_nine_slice(name, sprite,
      border)` on `UiRenderer` records source rect (via SpriteId) + per-side
      borders, with tint per draw.
- [x] **P1 — Vector icon library (Phosphor, MSDF).** Curated `PhosphorIcon`
      enum (regular/line weight, MIT TTF vendored under `assets/fonts/phosphor/`)
      rendered through a dedicated MSDF icon atlas that reuses the text MSDF
      generator/pipeline (extracted `MsdfTextureGpu` GPU mirror; icon atlas keyed
      `PHOSPHOR_FONT_ID`, `ref_px = 64`). API: `DrawList::icon_msdf(rect, icon,
      tint)` (fit-centred into the rect via `fit_centered`, transform/tint/clip
      aware, rotation-capable) and the stateless `Icon::new(PhosphorIcon).tint(..)
      .draw(rect, list)` widget. Adopted by the `NumberInput` `+`/`−` steppers.
      Behind the default-on `phosphor-icons` feature; widgets fall back to text
      glyphs when it's off.
- [ ] **P2 — Gradient helpers** (linear/radial). Per-vertex color exists but
      no constructor.
- [ ] **P2 — Text outline / shadow** (`UiTextOutline`, `UiTextShadow`).

---

## Widgets

- [x] **P0 — Dropdown / combo / select.** `Dropdown<'a>` + caller-owned
      single-owner `DropdownState`/`DropdownId` (one open at a time, like
      `FocusState`). Button drawn inline; the open list floats in a `Popup`
      layer pushed at frame-top (blocks clicks underneath). Click/Esc/
      click-outside close, hover highlight, selected highlight, scroll-clipped
      past `max_visible`. `src/widgets/dropdown.rs`. Keyboard nav + Tab-focus
      deferred (P1 below).
- [x] **P1 — Dropdown keyboard nav.** Arrow Up/Down + Enter to select, and
      register the dropdown in `FocusState` so Tab reaches it (open with
      Space/Enter). Same-frame open on keyboard activation (geom set immediately
      so the list appears without a one-frame delay). `Dropdown::draw` takes
      `&mut DrawContext` (which bundles `DrawList` + `FocusState` + `Theme` +
      `InputState` + screen dimensions). `key_up`/`key_down`/`key_space` added
      to `InputState`.
- [x] **P0 — ScrollView / scroll container** (general — `ScrollView` widget
      with caller-owned `ScrollState`, vertical+horizontal scroll, wheel
      input, draggable thumb, lives in `src/widgets/scroll_view.rs`. `Table`
      now uses it).
- [x] **P0 — Modal / dialog / popup layer** with z-order stacking and input
      gobbling. `LayerStack` in `src/layer.rs` plus
      `UiContext::modal_begin`/`modal_end`/`popup_begin`/`popup_end`.
      `InputState::mouse_consumed` tracks layer-dispatch capture.
- [x] **P0 — Popup / portal layer** for dropdowns, context menus, tooltips.
      `LayerStack::push_popup`/`push_tooltip`. Tooltip refactored to render
      onto its own layer via `TooltipLayer::draw_into_layers`.
- [x] **P0 — Image / sprite widget** with sizing/aspect/tinting/UV-rect.
      `Image` (`src/widgets/image.rs`) draws a `SpriteId` or string key into a
      dest box with `ImageFit` (Stretch/Contain/Cover/ScaleDown/None),
      `ImageAlign`, tint, and automatic UV cropping for `Cover` (via
      `DrawList::image_cropped`). Natural size supplied by the caller (from
      `UiRenderer::image_size`); aspect fits fall back to Stretch without it.
- [x] **P1 — Image / icon button.** `ImageButton`
      (`src/widgets/image_button.rs`) layers the `Image` widget (full
      `ImageFit`/`ImageAlign`/tint) over `Button`-style chrome with
      hover/press/disabled feedback, returning a click bool like `Button`.
      `.bare()` drops the chrome (image is the hit target, overlay-only
      feedback); `.padding()` insets the image. Disabled dims via overlay so
      string-key sources without tint still read as disabled.
- [x] **P1 — Radio button group.** `RadioGroup<'a>`
      (`src/widgets/radio.rs`) draws a mutually-exclusive option set from vector
      primitives (dot = `input_background` fill + `input_border` ring; selected
      adds an `accent` inner dot — no atlas assets needed). Caller owns the
      selected index: `draw(selected, rect, ctx) -> Option<usize>` returns the
      new index on change (click or, while focused, arrow keys). Builders:
      `new(&[&str])`, `.focusable(FocusId)` (one Tab stop for the whole group +
      arrow nav: Up/Down vertical, Left/Right horizontal, clamped no-wrap),
      `.horizontal()` (cells sized to measured label width), `.spacing(px)`.
      Façade verb `UiContext::radio_group(options, selected) -> usize`
      auto-places/advances like `checkbox`.
- [x] **P1 — Tree view / collapsing header.** `TreeNode`
      (`src/widgets/tree.rs`) draws one row — a disclosure triangle + indented
      label for *branches*, a terminal *leaf* otherwise — against a `Rect`/
      `DrawContext`. **Action-icon slots** (`with_leading`/`with_trailing` taking
      `&[TreeAction]`, sprite or string-key via `TreeIcon`) give the
      scene/layer-outliner shape: a leading visibility toggle + right-aligned
      rename/delete, each its own hit target — clicking one returns its
      `TreeAction::id` via `TreeNodeOutput::action` and does *not* select/expand.
      Interaction: the disclosure triangle toggles; the label/body selects (and
      *also* toggles only with `with_toggle_on_label`); actions fire alone.
      Caller-owned `TreeState` holds the expanded set + single-owner selection
      (`select`/`is_selected`/`toggle`/`set_expanded`/`collapse_all`);
      `with_default_open` expands a node the first time its `TreeId` is seen.
      Highlight spans the full row width; honors `mouse_consumed`. Façade
      `UiContext::tree_row(id, TreeNode)` → `TreeNodeOutput` is the rich verb;
      `tree_node`/`tree_node_open` (whole-row toggle) + `tree_leaf` are the
      no-icon convenience path, all with automatic per-depth indentation +
      auto-advance and `tree_pop`, backed by `UiState::tree`. Keyboard arrow-nav
      landed with the keyboard-navigation P1 (see below): `TreeState` nav-ring +
      `begin_frame`/`register_nav`/`end_frame(focused)`.
- [x] **P1 — Number input / spin box** with validation. `NumberInput`
      (`src/widgets/number_input.rs`) wraps a `TextInput` (inheriting cursor /
      selection / clipboard) around an `f64` value: parses + clamps to
      `[min, max]`, with +/- step buttons, mouse-wheel stepping, and Up/Down
      arrow stepping (both gated on focus so they never hijack page scroll or
      collide with text editing — single-line `TextInput` ignores Up/Down).
      `decimals` sets precision (`0` = integer). Text is sanitised to numeric
      characters while editing; the value owns the text when unfocused (external
      changes win), and `Enter` canonicalises. Façade
      `UiContext::number_input(id, &mut f64, min, max, step, decimals, w)`.
- [x] **P1 — Drag handle / window-mover.** `DragHandle`
      (`src/widgets/drag_handle.rs`) is a draggable grab-zone drawn against a
      `Rect`/`DrawContext`. It claims a caller-owned `DragCapture` (keyed by a
      stable `DragId`) on press so it can't fight sliders/scroll-thumbs/adjacent
      handles, and reports the per-frame pointer movement as
      `DragHandleOutput { dragging, started, released, delta }`. The delta is
      *consumed from* `InputState::drag_delta` — i.e. it composes with a
      caller-owned `DragTracker` (the "uses both" pattern the drag modules
      describe): run `DragTracker::update` each frame, then thread one shared
      `DragCapture` into every handle. `DragHandle::new()` draws title-bar chrome
      (panel background + hover/drag highlight + a centred grip glyph or a
      left-aligned `with_label`); `bare()` is a chrome-less pure hit-zone.
      `drag_rect(id, cap, handle_rect, &mut target, ctx)` is the convenience that
      applies the delta straight to a target `Rect`. Honors `mouse_consumed`.
      (No `UiContext` flow verb: a free-moving window is absolute-positioned, so
      it doesn't fit the auto-advance layout flow — use the widget directly with
      `UiState::drag` as the shared capture.)
- [x] **P1 — List / grid / virtualized list.** `List` (`src/widgets/list.rs`)
      is a general virtualized iterator over a flat `count` of items: it sets
      `ScrollState::content_size` and drives `ScrollView` (vertical), culling to
      the visible index range so a 10k-item collection costs a screenful. `List`
      owns no data — the caller passes `count` and a content closure
      `FnMut(&mut DrawList, Rect, ListItem)`; the widget draws the row chrome
      (selection/hover/zebra) and handles interaction, the closure fills the
      cell. Single column is a list; `.columns(n)` packs an `n`-wide grid
      (left-to-right, top-to-bottom) with `.with_gap(col, row)`. Selection is
      caller-owned in `ListState` (scroll + selected set + keyboard cursor):
      `SelectionMode::{None, Single, Multi}` — plain click replaces, Ctrl-click
      toggles, Shift-click selects the inclusive range from the anchor. With
      `.focused(true)`, arrow keys move the cursor (Up/Down by a column, Left/
      Right within a grid row), Home/End jump, Enter/Space activate, and the
      cursor auto-scrolls into view; edges are debounced like `Tree`. Returns
      `ListOutput { clicked, activated, hovered, mouse_over_content }`. Honors
      `mouse_consumed`. Like `Table`/`ScrollView` it takes a raw `&mut
      InputState` (it consumes the wheel), not a `DrawContext`. No `UiContext`
      façade yet (raw widget first, as Tree shipped).
- [ ] **P2 — Color picker.**
- [ ] **P2 — Separator / divider.**
- [ ] **P2 — Toast / notification / banner.**
- [ ] **P2 — Group / titled panel** (workshop equivalent of `UiWindow`).
- [x] **P1 — Tooltip hover delay actually works.** `TooltipLayer::tick(dt,
      input)` accumulates hover time per region; `is_visible()` only
      returns true once the configured `with_delay_ms` has elapsed.
- [x] **P1 — Slider drag capture identity.** `DragCapture`/`DragId`
      (`src/widgets/drag.rs`) arbitrate a single drag owner across the UI.
      `Slider::draw` now takes `id: DragId` + `&mut DragCapture` instead of a
      caller `bool`, claims the drag only when the capture is free, and updates
      its value only while it owns the capture (also honors `mouse_consumed`).
- [x] **P1 — Checkbox fallback rendering** when icon textures aren't loaded.
      `Checkbox` now defaults to a theme-driven vector box + contrast checkmark
      (no atlas assets, never blank). Opt into textures via
      `Checkbox::with_icons(SpriteId, SpriteId)` (pre-resolved, guaranteed
      non-blank) or `with_icon_keys(&str, &str)` for knowingly-preloaded keys.

---

## Layout

- [x] **P1 — Min/max size constraints.** `Constraint { min, max }`
      (`Constraint::min`/`max`/`between`, CSS semantics: min overrides max)
      clamps a resolved dimension orthogonally to `SizeSpec`. `Size` gained
      `min_width`/`max_width`/`min_height`/`max_height`; `VStack`/`HStack` gained
      `.constrain(Constraint)` applied to the last-added child. (Single-pass for
      `Fill` — a clamped fill child doesn't redistribute slack.)
- [x] **P1 — Per-child alignment within stack cells** (Center/Start/End on
      cross axis). `CrossAlign` enum (Start/Center/End/Stretch), `align()`
      builder on VStack and HStack. VStack respects alignment on the width
      axis, HStack on the height axis. `src/layout.rs`.
- [x] **P1 — Content-driven children.** The concrete gap (`content_size` always
      0.0 for `Fill`/`Percent`) is addressed by per-child alignment: `Fill` and
      `Percent` children now have proper `cross_size` control via alignment, and
      `Stretch` fills the cross-axis span as before. Full generic-child content
      driving (passing `impl LayoutNode` into stacks instead of raw pixel sizes)
      deferred to P2 as a larger API refactor. `src/layout.rs`.
- [x] **P1 — Z-order / layers** (required for popups/modals). `LayerStack`
      provides ordered Modal/Popup/Tooltip layers; `UiRenderer::render_layers`
      renders base → layers in order.
- [ ] **P2 — Weighted children** beyond equal-share `Fill`.
- [ ] **P2 — Wrap / flow layout** (inventory grids, mod lists).
- [ ] **P2 — Stable node IDs in `LayoutResult`** instead of positional indices
      (today `Vec<Rect>` indexed numerically, fragile under reordering).
- [ ] **P2 — Borrow/arena API for `LayoutResult.rects`** to avoid per-frame
      heap allocations.

---

## Input & Focus

- [x] **P0 — Real focus model.** Caller-owned `FocusState`/`FocusId`
      (mirrors `DragCapture`/`DragId`) is the single focus owner: at most one
      widget focused at a time. `TextInput.draw(id, &mut focus, …)` registers
      itself in the draw-order Tab ring and requests focus on click; Tab /
      Shift-Tab cycle, Esc and click-elsewhere blur, only the focused input
      consumes keys. `TextInput.focused: bool` removed. Modal-scoped Tab
      trapping deferred (see below).
- [x] **P1 — Modal/popup-scoped Tab trapping.** Tab cycling is scoped to the
      active layer via `FocusState::register_layer(id, layer_idx)` and
      `end_frame(Some(layer_idx))`. Base layer focusables are excluded from
      Tab order when a modal/popup is active. Click-to-focus already respects
      `mouse_consumed`. `DrawContext.register_focus(id)` auto-scopes based on
      `DrawContext.active_layer`. (`src/widgets/focus.rs`,
      `src/widgets/mod.rs`)
- [x] **P0 — Full key event model.** `InputState` now has `key_left`,
      `key_right`, `key_home`, `key_end`, `key_delete`, `shift_pressed`,
      `ctrl_pressed`. Arrows/Home/End/Delete on physical keys; Shift/Ctrl as
      held modifiers. Ctrl+A/X/C/V handled via ASCII control codes in
      `text_input` or `ctrl_pressed` + letter. Blocks usable text editing.
- [x] **P0 — Hit-testing respects clip stack and z-order.** Layer-aware
      input dispatch via `InputState::mouse_consumed` +
      `LayerStack::input_for_layer`/`input_for_base`. `is_hovered()` honors
      the flag automatically. (Note: existing widgets that call
      `rect.contains(input.mouse_x, ...)` directly should additionally AND
      with `!input.mouse_consumed` — `Table` and the example already do.)
- [x] **P1 — Multi-button mouse** (right click, middle click) for context
      menus. `InputState` gained `mouse_right_down/clicked/released` and
      `mouse_middle_down/clicked/released`. Edge fields (`clicked`/`released`)
      are cleared by `end_frame` and zeroed by `consumed()`; held-state (`down`)
      passes through. Example wires `MouseButton::Right` and `MouseButton::Middle`
      from the winit event. 5 tests. (`src/lib.rs`)
- [x] **P1 — Double-click and click-and-hold distinction** (timestamps). Caller-
      owned `ClickTracker` (same pattern as `DragTracker`): `update(&mut
      InputState, time_secs: f64)` writes `InputState::mouse_double_clicked`
      (second press within `double_click_threshold`, default 450 ms; window
      resets after a double so a rapid third click starts fresh) and
      `InputState::mouse_held` (latches after `hold_threshold`, default 500 ms,
      and stays true until release). Both fields cleared by `end_frame` and
      zeroed by `consumed()`. 14 tests; example gains a "dbl/hold" demo button.
      (`src/click_tracker.rs`)
- [x] **P1 — Drag detection on `InputState`** (`is_dragging`, `drag_delta`)
      so widgets stop reinventing it. `InputState` gained `is_dragging: bool`
      and `drag_delta: [f32; 2]`; a caller-owned [`DragTracker`] (mirrors
      `DragCapture`/`ScrollState`/`FocusState`) holds the cross-frame press
      origin + click-vs-drag latch and writes those fields each frame via
      `update(&mut InputState)`. Press-to-drag threshold (default 4px,
      configurable); a still press never drags; `cancel()` aborts; `consumed()`
      and `end_frame()` clear the outputs. Complementary to `DragCapture`
      (ownership) — a window-mover uses both. (`src/drag_tracker.rs`, 13 tests;
      example has a "drag me" box.)
- [x] **P1 — Scroll wheel propagation/capture** with a "consumed" flag for
      overlapping scroll regions. `InputState::scroll_consumed` is set by any
      `ScrollView` that claims the wheel (even at a clamp boundary, so the event
      can't "bubble out" to an outer scrollable). 4 tests: basic consume,
      inner/outer nesting, cursor-outside-inner passes through. (`scroll_view.rs`)
- [x] **P1 — IME / composition** for CJK/accented input. Crate-level done:
      `InputState.preedit`/`preedit_cursor` + `TextInput` renders the inline
      underlined preedit spliced at the caret (`compose_preedit` in
      `src/widgets/text_input.rs`). Game-side winit plumbing
      (`WindowEvent::Ime` → preedit, `set_ime_allowed`/`set_ime_cursor_area`)
      is a follow-up — the game feeds no keyboard/text input to the UI yet.
- [x] **P1 — Keyboard navigation** (Tab/Space-to-activate, arrows in lists).
      Opt-in `.focusable(FocusId)` on `Button` / `Checkbox` / `Slider`: when set
      the widget joins the Tab ring (`ctx.register_focus`), draws a focus ring
      (new `Theme::focus_ring` + `DrawContext::draw_focus_ring`), requests focus
      on click, and is keyboard-operable while focused — Button/Checkbox activate
      on Space/Enter, Slider adjusts on arrows (Left/Down decrement, Right/Up
      increment by `step` or 1/20 range, clamped). `Slider::focusable` takes a
      `FocusId` distinct from its `DragId`. `Tree` gets arrow navigation via a
      `TreeState` nav-ring mirroring `FocusState`: `begin_frame(&InputState)`
      (rising-edge capture), `register_nav` per row, `end_frame(focused: bool)`
      — Down/Up move selection, Right expand-then-descend, Left collapse-then-
      ascend, Enter/Space toggle; gated on `focused` so it can't fight a focused
      text field. The whole tree is one Tab-stop (`TreeState::set_focus_id`). The
      `UiContext` façade wires it all: `text_button`/`checkbox` get auto-ids,
      `slider` reuses its DragId, tree verbs register one reserved `FocusId` and
      draw the ring on the selected row.
- [ ] **P2 — Controller / gamepad** input abstraction.
- [ ] **P2 — Explicit `Frame`/`Ui` builder** that consumes input and produces
      a draw list, instead of implicit `end_frame` that callers can forget.

---

## Text

- [x] **P0 — Real text measurement** via glyphon shaping. Centering today
      uses `len() * font_size * 0.5` in 6+ places; broken for proportional
      fonts, multi-byte UTF-8, non-ASCII. (Teardown ships `UiGetTextSize` for
      this reason.)
- [x] **P0 — Text selection.** Caret state, shift+arrow selection,
      click-to-position-cursor, selection highlight rendering, Ctrl+A select all,
      proper backspace/delete. (`src/widgets/text_input.rs`)
- [x] **P0 — Copy/paste / clipboard hooks.** Closure-based clipboard API on
      `TextInput` (`clipboard_get`/`clipboard_set`). Users wire the platform
      clipboard (e.g. `arboard`). Cut/copy/paste (Ctrl+X/C/V) when closures are
      set.
- [x] **P0 — Font system.** Runtime font loading (`load_font_file`/
      `load_font_bytes` → `FontHandle`), per-`TextBlock` font selection
      (`with_font`/`with_font_opt`), bold/italic/weight (`TextBlock::bold()`/
      `italic()`/`with_weight()`/`with_style()`, threaded through the shape
      cache + measurement), a bundled default font (Noto Sans, behind the
      default-on `bundled-font` feature → deterministic `Family::SansSerif` via
      `register_bundled_fonts`), `Theme.font` driving every widget, and the
      Teardown `UiFont(family,size)` push/pop font stack on `UiContext`
      (`font`/`font_size`/`font_family`/`bold`/`italic`/`text_line`).
      cosmic-text's script/glyph fallback is automatic. (Synthetic bold/oblique
      for absent faces is out of scope — cosmic-text selects real faces only.)
- [x] **P1 — Multi-line `TextInput` / textarea.** `TextInput::with_multiline`
      (and the `UiContext::text_area(id, buf, ph, w, rows)` façade): Enter inserts
      `\n`, the value wraps to the field width, Up/Down navigate visual lines with
      a sticky column, Home/End are line-relative, selection renders per line, and
      the field clips + autoscrolls vertically to keep the caret visible
      (`scroll_offset`). Line-aware caret/click via `text_caret_layout`/`CaretPos`.
- [x] **P1 — Text wrapping policy.** `TextBlock::with_wrap(WrapMode)` —
      `None`/`Word`/`Glyph`/`WordOrGlyph` (default `WordOrGlyph` preserves prior
      rendering). Single-line `TextInput` now uses `WrapMode::None` (no silent
      wrap-to-hidden-second-line); multiline uses `WordOrGlyph`.
- [x] **P1 — Ellipsis on overflow.** `TextBlock::with_ellipsis` lays the text
      on one line and truncates with an `…` reserved at the right edge
      (`ellipsize_to_width` in `src/text.rs`); opt-in, no-op when the text fits.
- [x] **P1 — Cursor x-position from real shaping**, not the same broken
      `len*0.5` formula.
- [ ] **P2 — Password / masked input.**
- [ ] **P2 — RTL / bidi exposure** (glyphon supports it; no public knob).

---

## Theming / Styling

- [x] **P0 — DPI / scale factor** propagated through renderer; affects
      glyphon resolution, vertex output, layout. Teardown's `UiScale` exists
      (~1.2k mod calls). `UiRenderer::render`/`render_layers` take a
      `scale_factor`; the ortho is built from the logical size while the
      framebuffer stays physical (MSDF text self-sharpens via `fwidth`).
- [x] **P1 — Multiple fonts / sizes / weights** (see font system above) —
      per-`TextBlock` family/size/weight/style, `Theme.font`, and the
      `UiContext` font stack all land this.
- [x] **P1 — Per-widget style override** without copying the whole `Theme`.
      Added a no-clone **style resolver** (`src/style.rs`): `StyleKey` (one
      variant per theme field + `Custom(u64)` name-hash), `StyleValue`
      (`Color`/`Scalar`), `StyleOverlay` (caller-owned sparse override set), and
      `StyleResolver` (precedence: overlay → theme). Every widget now resolves
      style through the resolver — `DrawContext` carries an optional
      `&StyleOverlay` (`ctx.color(key)`/`ctx.scalar(key)`, set via
      `DrawContext::with_style(&overlay)`); bare-`&Theme` widgets/free-fns
      (`Panel`, `ProgressBar`, `Tabs`, `Table`, `Tooltip`, `List`, `ScrollView`,
      `label`/`title`/…) now take `&StyleResolver`. `UiContext` gained a scoped
      style stack (`set_style`/`set_style_color`/`set_style_scalar`/`clear_style`,
      pushed/popped with `push`/`pop` like the tint/font stacks) so a subtree
      restyles without a theme clone. No-overlay path is value-identical to the
      old `theme.<field>` reads.
- [x] **P1 — Extensible theme** (typed style map / `HashMap<StyleKey,
      StyleValue>`) so custom widgets don't need core changes. `Theme` keeps its
      flat typed fields as the source of truth and bridges them through
      `Theme::get`/`set(StyleKey, StyleValue)`; a `custom: HashMap<u64,
      StyleValue>` map holds mod-defined keys. `StyleKey::custom(name)` addresses
      them by FNV-1a name-hash (no global interner). A custom widget can carry
      its own keys via an overlay or `Theme::register_style`.
- [x] **P1 — Hover/press animation clock + transitions / easing.**
      egui-style "animate toward the resolved color": each frame a widget
      resolves its discrete target color and the clock eases the *displayed*
      color toward it, re-basing when the target changes (no muddy multi-state
      blend). `src/animation.rs`: `AnimationState` (caller-owned, dt-driven,
      keyed by `(u64 id, AnimSlot)`), `Easing{Linear,EaseIn,EaseOut,EaseInOut}`,
      and public `ease`/`lerp`/`lerp_color` (endpoint-snapping). Duration is a
      themeable scalar — `Theme::animation_duration` (default `0.12`) /
      `StyleKey::AnimationDuration`, overridable per-subtree via `StyleOverlay`;
      `0.0` snaps. Seam: `DrawContext::with_animations(&mut AnimationState)` +
      `ctx.animate_color(id, slot, target)` / `animate_scalar(...)` (resolve
      duration, ease-out; return `target` unchanged when no state is attached →
      byte-identical to the instant path). Raw widgets opt in with `.animated(id)`
      — adopted in **Button** (bg + border), **Checkbox** (box fill + hover
      overlay alpha), **Tabs** (per-tab bg + label, sub-key
      `base_id.wrapping_add(i)`). Façade auto-wires it: `UiState.anim` ticked by
      `begin_frame(input, theme, dt)`, and `text_button`/`checkbox` pass
      `.animated(auto_id)` + the shared state, so interactive-mode apps get
      hover/press easing for free. Hard invariant held: with no state (or
      `dt`/`duration == 0`) every drawn value is byte-identical, so existing
      tests + the gallery stayed green. Gallery: "Hover animation (easing)" ramp
      samples the ease-out curve at t∈{0,.25,.5,.75,1}. Other raw widgets adopt
      the same `.animated(id)` pattern as needed.
- [ ] **P2 — Theme stack** (push tint/color), tied to A5/D8 above.
- [ ] **P2 — Move semantic policy out of theme.** `progress_fill`/`_low`
      colors encode "low = red"; should be a thresholds struct or callback.

---

## Game / Teardown-Specific

- [x] **P0 — Lua-binding-friendly facade** (`UiContext` with state stack)
      that backs `UiText`/`UiTextButton`/`UiImageBox`/`UiSlider`/etc. as
      stateful immediate-mode calls. `UiContext` (push/pop,
      translate/rotate/scale, align/center, color/color_filter, place_rect,
      quad/rounded_rect/text_block) plus the font stack landed earlier; the
      interactive mode (`UiContext::interactive`/`interactive_layers` +
      caller-owned `UiState`) now provides the auto-advancing stateful verbs
      `text`/`text_button`/`slider`/`checkbox`/`image_box`/`text_input`.
      Remaining: UI sound hooks (tracked separately below).
- [x] **P0 — World-space UI** (`UiWorldToPixel`/`UiWorldToScreen`) for
      in-world labels, damage numbers, health bars over NPCs.
      `projection::world_to_screen`/`world_to_screen_na` project a world
      point to UI pixel space (None behind the camera).
- [ ] **P1 — UI sound hooks.** `UiSound`/`UiSoundLoop` and button
      hover/press sounds. **Deferred to the integrating app** (decision
      2026-06): this library is render-only and has no audio backend, so the
      app owns sound — it already gets the interaction edges it needs from the
      widget return values + `HitZoneOutput` (`clicked`/`pressed`/`hovered`/…)
      to trigger its own SFX. Re-open only if a built-in hook proves necessary.
- [~] **P1 — Mod-friendly registration** of custom widgets/styles.
      `register_style(name, value)` landed: `Theme::register_style(&mut self,
      name, StyleValue)` + `Theme::style(name) -> Option<StyleValue>` store/read
      custom keys by name-hash (and `StyleOverlay` can carry them per-subtree).
      `register_widget(name, draw_fn)` is **deferred until Lua integration**
      (decision 2026-06) — widgets have heterogeneous signatures and there's no
      uniform draw-fn contract yet, and the right shape is hard to know without a
      concrete modding consumer driving the requirements. Design it against a
      real `register_widget` call site (the Lua binding layer) rather than in the
      abstract.
- [x] **P1 — `UiMakeInteractive` / hit-zones independent of draw** for
      sensors over 3D things. `HitZone` (`src/widgets/hit_zone.rs`) is the
      deliberate draw-free widget: `HitZone::new().test(rect, &input) ->
      HitZoneOutput` reports `hovered`/`pressed`/`clicked`/`released`/
      `right_clicked`/`middle_clicked`/`double_clicked`/`held`/`scroll_delta`/
      `local_pos` over a screen-space `Rect` without touching any `DrawList` — so
      it lays over regions this UI didn't paint (a 3D viewport, a
      `world_to_screen` rect). Takes a plain `&InputState` (nothing for a
      `DrawContext` to carry); honors `InputState::mouse_consumed` for layer
      capture; reports only (never sets `mouse_consumed`) like the other
      per-layer widgets — gate world-picking on `!out.hovered`. `.enabled(false)`
      makes it inert. Façade verbs: `UiContext::hit_zone(w, h)` (flow-placed
      cell, auto-advances) and `UiContext::hit_zone_at(rect)` (explicit screen
      rect, no advance) for absolute sensors.
- [ ] **P2 — Cursor state control** (`UiSetCursorState`, I-beam over text).
- [ ] **P2 — Backdrop blur** (`UiBlur`) for menu screens.
- [ ] **Out of scope but don't block:** depth-aware `DrawSprite`/`DrawLine`
      in 3D world space — keep UI overlay vs. world overlay passes
      separable.

---

## Testing & Docs

- [x] **P1 — Widget tests.** Every widget module has `#[cfg(test)]` with
      headless `DrawList` tests; `text_input.rs` alone has 20+ tests, and
      `draw_list.rs`, `scroll_view.rs`, `button.rs`, `dropdown.rs`, etc. all
      have test suites. The TODO description was stale — tests existed at the
      time the repo was extracted from citybuilder and audits hadn't caught
      them. (Confirmed 2026-04-27: every `src/widgets/*.rs` except `mod.rs`
      has `#[cfg(test)]`.)
- [x] **P1 — `examples/` directory with at least one runnable wgpu
      example.** `examples/hello_ui.rs` opens a window and renders a panel +
      button + icon + nine-slice + text via `UiRenderer`.
- [ ] **P2 — Rustdoc on all public types.** Several lack it: `Vertex`,
      `IconDraw`, `NineSliceDraw`, `LayoutResult`, `StackChild`,
      `DrawContext::*`.
- [ ] **P2 — README quickstart, widget gallery, architecture overview.**
- [ ] **P2 — Bench suite** (`benches/`) for hot paths (text shaping, draw
      list construction, layout).

---

## Suggested 1.0 Roadmap (in priority order)

1. **Renderer + atlas** — ship a working `UiRenderer` and texture atlas; the
   crate currently can't actually draw quads/sprites/nine-slices.
2. **Rounded rects + lines + clip stack** — foundational primitives that
   unlock proper-looking UI and scroll views.
3. **Matrix-stack `UiContext`** (push/pop/translate/align/color) — required
   for Teardown port, dramatically improves widget ergonomics.
4. **ScrollView + modal/popup layer** — enables dropdown, color picker,
   drag handle, etc.
5. **Real text editing**: focus model, full key events, selection,
   clipboard.
6. **Font system + DPI scaling.**
7. **Dropdown, Image, ImageButton** — closes biggest widget gaps.
8. **Real text measurement** (glyphon-backed) — fixes alignment everywhere.
9. **Layout: min/max, alignment, content-driven children.**
10. **Widget API unification + widget tests + runnable example.**

Beyond 1.0: world-space UI, sound hooks, mod registry, color picker,
collapsing header, blur backdrop, controller input.
