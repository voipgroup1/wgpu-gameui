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
- [ ] **P0 — Matrix / transform stack** (`UiPush`/`UiPop`/`UiTranslate`/`UiAlign`/
      `UiCenter`/`UiRotate`/`UiScale`). All widgets currently take absolute
      `Rect`s. This is the single biggest blocker for the Teardown UI port.
- [ ] **P0 — Color / tint stack** (`UiColor`/`UiColorFilter`) with sub-tree alpha
      multiplier so panels can fade in/out. Today vertex colors are baked.
- [x] **P0 — Clip / scissor stack.** `push_clip(rect)`/`pop_clip()` with draw
      commands grouped per clip stack. `Table::draw_cell` text is currently
      *not actually clipped* by `content_rect`.
- [ ] **P1 — Unify widget API around `DrawContext` + `Rect`.** Today some
      widgets are structs with `draw`/`draw_at`, some are unit structs with
      assoc fns, some are free functions. `DrawContext` exists in
      `src/widgets/mod.rs` but is unused.
- [x] **P1 — Replace `String` keys in draw commands with interned `IconId`/
      `SpriteId`/`u32` handles** produced by the atlas. (Both still accept
      string-keyed helpers for ergonomics; `icon_sprite`/`nine_slice_id` are the
      allocation-free path.)
- [ ] **P1 — Don't `unwrap()` glyphon errors in `TextRenderer::prepare`/
      `render`** (`src/text.rs:103,130`). Bubble as a typed `UiError`.
- [ ] **P1 — Cache glyphon `Buffer`s by content+size hash or pool them.**
      Currently a fresh buffer is built per `TextBlock` per frame.

---

## Draw Primitives

- [x] **P0 — Rounded rectangles.** `theme.border_radius` exists but is never
      used. Teardown's `UiRoundedRect` is widely used. Tessellate or SDF.
- [x] **P0 — Lines / strokes** (`line(p0, p1, thickness, color)`) with
      thickness/joins/caps/AA. Teardown's `DrawLine` is top-30. Also needed
      for slider tick marks, debug overlays.
- [ ] **P1 — Circles / arcs / ellipses** (`UiCircle`).
- [x] **P1 — Textured quad with explicit UV rect.** Atlas `AtlasRegion::uv()`
      drives icon UVs, with optional tint per draw.
- [x] **P1 — Nine-slice border metadata.** `register_nine_slice(name, sprite,
      border)` on `UiRenderer` records source rect (via SpriteId) + per-side
      borders, with tint per draw.
- [ ] **P2 — Gradient helpers** (linear/radial). Per-vertex color exists but
      no constructor.
- [ ] **P2 — Text outline / shadow** (`UiTextOutline`, `UiTextShadow`).

---

## Widgets

- [ ] **P0 — Dropdown / combo / select.** Universal in settings/mod menus.
- [ ] **P0 — ScrollView / scroll container** (general — today `ScrollState`
      lives only inside `Table`).
- [ ] **P0 — Modal / dialog / popup layer** with z-order stacking and input
      gobbling. Teardown's `UiModalBegin`/`UiModalEnd`/`UiWindow`.
- [ ] **P0 — Popup / portal layer** for dropdowns, context menus, tooltips.
      Tooltip currently relies on caller draw order
      (`src/widgets/tooltip.rs:135` — "call this at the end").
- [ ] **P0 — Image / sprite widget** with sizing/aspect/tinting/UV-rect
      (`UiImage`/`UiImageBox`).
- [ ] **P1 — Image / icon button** (`UiImageButton`/`UiButtonImageBox`).
- [ ] **P1 — Radio button group.**
- [ ] **P1 — Tree view / collapsing header.**
- [ ] **P1 — Number input / spin box** with validation. TextInput is raw text.
- [ ] **P1 — Drag handle / window-mover.**
- [ ] **P1 — List / grid / virtualized list.** Table is the only iterator.
- [ ] **P2 — Color picker.**
- [ ] **P2 — Separator / divider.**
- [ ] **P2 — Toast / notification / banner.**
- [ ] **P2 — Group / titled panel** (workshop equivalent of `UiWindow`).
- [ ] **P1 — Tooltip hover delay actually works.** `TooltipLayer.hover_delay_ms`
      is stored but never read; tooltips show immediately
      (`src/widgets/tooltip.rs:99,118,144`).
- [ ] **P1 — Slider drag capture identity.** Drag state is a caller-owned
      `bool`; multiple sliders can't disambiguate active drag
      (`src/widgets/slider.rs:67`).
- [ ] **P1 — Checkbox fallback rendering** when icon textures aren't loaded
      (`src/widgets/checkbox.rs:9,38`).

---

## Layout

- [ ] **P1 — Min/max size constraints.** `SizeSpec` is Fixed/Percent/Fill/Fit
      with no `Min`/`Max`/`between`.
- [ ] **P1 — Per-child alignment within stack cells** (Center/Start/End on
      cross axis). HStack always fills cross axis (`src/layout.rs:495`).
- [ ] **P1 — Content-driven children.** `VStack::child(height, width)` takes
      raw numbers, not actual widget content; `content_size` is always 0.0
      for `Fill`/`Percent` (`src/layout.rs:296-303`).
- [ ] **P1 — Z-order / layers** (required for popups/modals).
- [ ] **P2 — Weighted children** beyond equal-share `Fill`.
- [ ] **P2 — Wrap / flow layout** (inventory grids, mod lists).
- [ ] **P2 — Stable node IDs in `LayoutResult`** instead of positional indices
      (today `Vec<Rect>` indexed numerically, fragile under reordering).
- [ ] **P2 — Borrow/arena API for `LayoutResult.rects`** to avoid per-frame
      heap allocations.

---

## Input & Focus

- [ ] **P0 — Real focus model.** `TextInput.focused: bool` is set by the
      caller. No focus owner, Tab navigation, blur-on-click-elsewhere,
      Esc-to-blur. Multiple text inputs all activate at once.
- [ ] **P0 — Full key event model.** `InputState` has only
      `backspace_pressed`, `enter_pressed`, and a `text_input` string. No
      Shift/Ctrl/Alt/Cmd, arrows, Home/End, Delete, Ctrl+A/C/V/X. Blocks
      usable text editing.
- [ ] **P0 — Hit-testing respects clip stack and z-order.** A button under a
      modal currently still registers hover.
- [ ] **P1 — Multi-button mouse** (right click, middle click) for context
      menus.
- [ ] **P1 — Double-click and click-and-hold distinction** (timestamps).
- [ ] **P1 — Drag detection on `InputState`** (`is_dragging`, `drag_delta`)
      so widgets stop reinventing it.
- [ ] **P1 — Scroll wheel propagation/capture** with a "consumed" flag for
      overlapping scroll regions.
- [ ] **P1 — IME / composition** for CJK/accented input.
- [ ] **P1 — Keyboard navigation** (Tab/Space-to-activate, arrows in lists).
- [ ] **P2 — Controller / gamepad** input abstraction.
- [ ] **P2 — Explicit `Frame`/`Ui` builder** that consumes input and produces
      a draw list, instead of implicit `end_frame` that callers can forget.

---

## Text

- [x] **P0 — Real text measurement** via glyphon shaping. Centering today
      uses `len() * font_size * 0.5` in 6+ places; broken for proportional
      fonts, multi-byte UTF-8, non-ASCII. (Teardown ships `UiGetTextSize` for
      this reason.)
- [ ] **P0 — Text selection.** No caret state, shift+arrow selection, or
      click-to-position-cursor. Cursor hardcoded to end of value
      (`src/widgets/text_input.rs:94`).
- [ ] **P0 — Copy/paste / clipboard hooks.**
- [ ] **P0 — Font system.** Hardcoded `Family::SansSerif`
      (`src/text.rs:74`). No font loading, bold/italic, `UiFont(path,size)`,
      or font fallback. `UiFont` is one of the most-called UI fns in
      Teardown (~2k mod calls).
- [ ] **P1 — Multi-line `TextInput` / textarea.** Enter is consumed as a
      separate event, never inserted.
- [ ] **P1 — Text wrapping policy.** `max_width` set, but smaller-than-text
      causes silent truncation. `UiWordWrap` exists in Teardown.
- [ ] **P1 — Ellipsis on overflow.**
- [x] **P1 — Cursor x-position from real shaping**, not the same broken
      `len*0.5` formula.
- [ ] **P2 — Password / masked input.**
- [ ] **P2 — RTL / bidi exposure** (glyphon supports it; no public knob).

---

## Theming / Styling

- [ ] **P0 — DPI / scale factor** propagated through renderer; affects
      glyphon resolution, vertex output, layout. Teardown's `UiScale` exists
      (~1.2k mod calls).
- [ ] **P1 — Multiple fonts / sizes / weights** (see font system above).
- [ ] **P1 — Per-widget style override** without copying the whole `Theme`.
- [ ] **P1 — Extensible theme** (typed style map / `HashMap<StyleKey,
      StyleValue>`) so custom widgets don't need core changes.
- [ ] **P1 — Hover/press animation clock + transitions / easing.** Today
      colors switch immediately.
- [ ] **P2 — Theme stack** (push tint/color), tied to A5/D8 above.
- [ ] **P2 — Move semantic policy out of theme.** `progress_fill`/`_low`
      colors encode "low = red"; should be a thresholds struct or callback.

---

## Game / Teardown-Specific

- [ ] **P0 — Lua-binding-friendly facade** (`UiContext` with state stack)
      that backs `UiText`/`UiTextButton`/`UiImageBox`/`UiSlider`/etc. as
      stateful immediate-mode calls. The crate today is "give me a Rect,
      build draw commands"; the Teardown layer is the missing wrap.
- [ ] **P0 — World-space UI** (`UiWorldToPixel`/`UiWorldToScreen`) for
      in-world labels, damage numbers, health bars over NPCs.
- [ ] **P1 — UI sound hooks.** `UiSound`/`UiSoundLoop` and button
      hover/press sounds.
- [ ] **P1 — Mod-friendly registration** of custom widgets/styles
      (`register_widget(name, draw_fn)`, `register_style(name, value)`).
- [ ] **P1 — `UiMakeInteractive` / hit-zones independent of draw** for
      sensors over 3D things.
- [ ] **P2 — Cursor state control** (`UiSetCursorState`, I-beam over text).
- [ ] **P2 — Backdrop blur** (`UiBlur`) for menu screens.
- [ ] **Out of scope but don't block:** depth-aware `DrawSprite`/`DrawLine`
      in 3D world space — keep UI overlay vs. world overlay passes
      separable.

---

## Testing & Docs

- [ ] **P1 — Widget tests.** Today only `layout.rs` has tests (4 of them).
      No `#[cfg(test)]` in any `widgets/*.rs`. The design is testable with
      mock `InputState` + draw-list inspection (slider drag, table scroll,
      text-input editing).
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
