//! Headless offscreen render of all widgets → PNG for visual inspection.
//!
//! Ignored by default (needs a GPU adapter). Run with:
//! ```
//! cargo test -p wgpu-gameui --test widget_gallery -- --ignored --nocapture
//! ```
//! Writes `test_output/widget_gallery.png`.
//!
//! Layout is driven by [`Flow`] — a left-to-right, wrapping grid of labeled
//! cells. Each preview reserves a cell (which draws its label) and gets back a
//! content `Rect` to draw into, so adding a widget is one `flow.cell(...)` call
//! plus the widget's own draw call — no hand-placed coordinates.

use wgpu_gameui::layout::{Flow as LayoutFlow, HStack, LayoutNode, MainAlign, Rect};
use wgpu_gameui::{
    Button, Checkbox, ColorPicker, ColumnWidth, DragCapture, DragHandle, DrawContext, DrawList,
    Dropdown, DropdownState, Easing, FocusState, HitZone, Hsva, ImageButton, ImageFit, InputState,
    LayerStack, List,
    ListItem, ListState, NumberInput, ProgressBar, RadioGroup, ScrollState, ScrollView,
    SelectionMode, Separator, Slider, StyleKey, StyleOverlay, StyleResolver, Table, TableCell,
    TableColumn,
    Tabs, TextAlign, TextBlock, TextInput, TextSpan, Theme, TooltipContent, TooltipLayer,
    TreeAction, TreeNode, TreeState, UiContext, UiRenderer, UiState, ease, lerp_color,
};
#[cfg(feature = "phosphor-icons")]
use wgpu_gameui::{Icon, PhosphorIcon};

/// Convenience: build a DrawContext for a single draw call in the gallery.
fn ctx<'a>(
    list: &'a mut DrawList,
    focus: &'a mut FocusState,
    theme: &'a Theme,
    input: &'a InputState,
) -> DrawContext<'a> {
    DrawContext::new(list, focus, theme, input, W as f32, 600.0)
}

const W: u32 = 800;
const LABEL_H: f32 = 16.0;
const LABEL_SIZE: f32 = 11.0;
/// Rough advance width per character at `LABEL_SIZE`, used so a long label
/// reserves enough horizontal room to not collide with the next cell.
const LABEL_CHAR_W: f32 = 6.0;

/// A wrapping grid of labeled preview cells.
///
/// `cell` reserves a `w`×`h` content box, draws its label just above it, and
/// returns the content `Rect`. Cells flow left-to-right and wrap when they run
/// past `max_x`. `section` breaks to a new row and draws a header.
struct Flow {
    x0: f32,
    y0: f32,
    max_x: f32,
    cur_x: f32,
    cur_y: f32,
    row_h: f32,
    col_gap: f32,
    row_gap: f32,
}

impl Flow {
    fn new(x0: f32, y0: f32, max_x: f32) -> Self {
        Self {
            x0,
            y0,
            max_x,
            cur_x: x0,
            cur_y: y0,
            row_h: 0.0,
            col_gap: 22.0,
            row_gap: 16.0,
        }
    }

    /// Break to a new row and draw a section header.
    fn section(&mut self, list: &mut DrawList, title: &str) {
        if self.cur_x > self.x0 {
            self.cur_y += self.row_h;
        }
        // Extra gap above a header (except the very first one).
        if self.cur_y > self.y0 {
            self.cur_y += self.row_gap * 1.5;
        }
        self.cur_x = self.x0;
        self.row_h = 0.0;
        list.text(
            TextBlock::new(title, self.x0, self.cur_y)
                .with_size(15.0)
                .with_color(120, 180, 255),
        );
        self.cur_y += 24.0;
    }

    /// Reserve a labeled `w`×`h` content cell; returns the content rect.
    fn cell(&mut self, list: &mut DrawList, label: &str, w: f32, h: f32) -> Rect {
        // A cell is as wide as its content or its label, whichever is larger,
        // so labels never overlap the neighbouring cell.
        let cell_w = w.max(label.chars().count() as f32 * LABEL_CHAR_W);
        if self.cur_x + cell_w > self.max_x && self.cur_x > self.x0 {
            self.cur_x = self.x0;
            self.cur_y += self.row_h + self.row_gap;
            self.row_h = 0.0;
        }
        list.text(
            TextBlock::new(label, self.cur_x, self.cur_y)
                .with_size(LABEL_SIZE)
                .with_color(150, 160, 180),
        );
        let content = Rect::new(self.cur_x, self.cur_y + LABEL_H, w, h);
        self.cur_x += cell_w + self.col_gap;
        self.row_h = self.row_h.max(LABEL_H + h);
        content
    }

    /// Reserve at least `content_h` of vertical space (measured from the last
    /// cell's content-rect top) for the current row. Use this for content that
    /// *paints taller than the `cell` rect it was handed* — an open `Dropdown`
    /// overlay or an auto-advancing `UiContext` verb stack — so the following
    /// row starts below it instead of underneath it.
    fn reserve(&mut self, content_h: f32) {
        self.row_h = self.row_h.max(LABEL_H + content_h);
    }

    /// The y just below all content drawn so far.
    fn bottom(&self) -> f32 {
        self.cur_y + self.row_h
    }
}

fn solid_with_border(size: u32, fill: [u8; 4], border: [u8; 4], thickness: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let on_border =
                x < thickness || y < thickness || x >= size - thickness || y >= size - thickness;
            let c = if on_border { border } else { fill };
            let idx = ((y * size + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

#[test]
#[ignore = "needs a GPU adapter; writes a PNG for manual inspection"]
fn render_widget_gallery() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter available");

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("gallery device"),
            ..Default::default()
        },
        None,
    ))
    .expect("request device");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let font_system = wgpu_gameui::shared_font_system();
    let mut ui = UiRenderer::new(&device, &queue, format, font_system.clone());

    // Real PNG art from assets/ — decoded through the same `load_image_file`
    // path the game uses, so the gallery doubles as a smoke test for it.
    let load = |ui: &mut UiRenderer, name: &str| {
        ui.load_image_file(format!("assets/{name}.png"))
            .unwrap_or_else(|e| panic!("load assets/{name}.png: {e:?}"))
    };
    let duck = load(&mut ui, "rubberduck");
    let ball = load(&mut ui, "soccerball");
    let car = load(&mut ui, "toycar");
    let board = load(&mut ui, "skateboard");
    let snow = load(&mut ui, "snowflake");
    let suitcase = load(&mut ui, "suitcase");

    // Synthetic sprites for primitives that show off tinting / nine-slice.
    let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
    let frame_sprite = ui.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
    let nine_slice_id = ui.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);

    // (Slider needs no assets — it renders procedurally from the theme.)

    let mut layers = LayerStack::new();
    let theme = Theme::default();
    let mut input = InputState::default();

    // Focus owner for the text inputs. Seed the first as focused so the rendered
    // PNG shows a caret; `begin_frame`/`end_frame` bracket the draws below.
    let mut focus = FocusState::new();
    focus.focus(0);
    focus.begin_frame(&input);

    // Dropdown owner, seeded OPEN so the PNG shows the floating option list.
    const DROPDOWN_ID: u64 = 100;
    const DROPDOWN_ITEMS: [&str; 4] = ["Red", "Green", "Blue", "Alpha"];
    let mut dropdowns = DropdownState::new();

    // Reserved by the flow inside the scope below, used afterwards. The scope
    // runs unconditionally, so deferred init is sound (and avoids a dead store).
    let tooltip_rect;
    let content_bottom;

    // =====================================================================
    // Build the base layer. The `list` borrow on `layers` is released at the
    // end of this scope so we can add tooltip layers afterwards.
    // =====================================================================
    {
        let list = layers.base_mut();

        list.text(
            TextBlock::new("wgpu-gameui Widget Gallery", 20.0, 16.0)
                .with_size(24.0)
                .with_color(255, 255, 255),
        );

        let mut flow = Flow::new(20.0, 56.0, (W as f32) - 20.0);

        // ---- Primitives -------------------------------------------------
        flow.section(list, "Primitives");

        let r = flow.cell(list, "Rounded rect", 120.0, 44.0);
        list.rounded_rect(r, 8.0, [0.25, 0.40, 0.65, 1.0]);

        let r = flow.cell(list, "Line", 90.0, 44.0);
        list.line(
            [r.x, r.y + r.height],
            [r.x + r.width, r.y],
            3.0,
            [0.95, 0.65, 0.25, 1.0],
        );

        let r = flow.cell(list, "Circle", 50.0, 50.0);
        list.circle(
            (r.x + r.width / 2.0, r.y + r.height / 2.0),
            r.width / 2.0,
            [0.30, 0.70, 0.40, 1.0],
        );

        let r = flow.cell(list, "Circle outline", 50.0, 50.0);
        list.circle_outline(
            (r.x + r.width / 2.0, r.y + r.height / 2.0),
            r.width / 2.0,
            3.0,
            [0.70, 0.30, 0.40, 1.0],
        );

        // Rect outline routes through the SDF chrome instance (transparent fill,
        // border-only band) when translate-only.
        let r = flow.cell(list, "Rect outline", 120.0, 44.0);
        list.rect_outline(r, 2.0, [0.55, 0.75, 0.95, 1.0]);

        // Rotated rounded rect: a non-translate transform falls back to the soup
        // tessellator, proving the rounded-rect primitive still draws correctly
        // off-axis (instanced fast path is translate-only).
        let r = flow.cell(list, "Rounded rect (rotated)", 120.0, 44.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.rounded_rect(
            Rect::new(-r.width / 2.0, -r.height / 2.0, r.width, r.height),
            8.0,
            [0.25, 0.40, 0.65, 1.0],
        );
        list.pop_transform();

        let r = flow.cell(list, "Nine-slice", 64.0, 44.0);
        list.nine_slice_id(nine_slice_id, r.x, r.y, r.width, r.height, [1.0; 4]);

        // Rotated nine-slice: unlike chrome, the instanced nine-slice bakes the
        // full affine into the instance (UV mapping is local-space), so rotation
        // is exact with no fallback — strictly more capable than the old soup.
        let r = flow.cell(list, "Nine-slice (rotated)", 64.0, 44.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.nine_slice_id(
            nine_slice_id,
            -r.width / 2.0,
            -r.height / 2.0,
            r.width,
            r.height,
            [1.0; 4],
        );
        list.pop_transform();

        let r = flow.cell(list, "Icon sprite", 40.0, 40.0);
        list.icon_sprite(ball, r.x, r.y, r.width, r.height, [1.0; 4]);

        let r = flow.cell(list, "Image", 40.0, 40.0);
        list.image(car, r, [1.0; 4]);

        let r = flow.cell(list, "Image (cropped)", 40.0, 40.0);
        list.image_cropped(snow, r, [0.0, 0.0, 0.5, 0.5], [1.0; 4]);

        let r = flow.cell(list, "Rect outline", 100.0, 32.0);
        list.rect_outline(r, 2.0, [0.70, 0.30, 0.40, 1.0]);

        // ---- Phosphor MSDF icons ---------------------------------------
        #[cfg(feature = "phosphor-icons")]
        {
            flow.section(list, "Icons (Phosphor MSDF)");

            // The full curated set at a single readable size.
            let set = [
                ("Plus", PhosphorIcon::Plus),
                ("Minus", PhosphorIcon::Minus),
                ("Check", PhosphorIcon::Check),
                ("X", PhosphorIcon::X),
                ("CaretUp", PhosphorIcon::CaretUp),
                ("CaretDown", PhosphorIcon::CaretDown),
                ("Eye", PhosphorIcon::Eye),
                ("EyeSlash", PhosphorIcon::EyeSlash),
                ("Trash", PhosphorIcon::Trash),
                ("Pencil", PhosphorIcon::PencilSimple),
                ("Gear", PhosphorIcon::Gear),
            ];
            for (label, icon) in set {
                let r = flow.cell(list, label, 32.0, 32.0);
                Icon::new(icon).draw(r, list);
            }

            // A few sizes of one icon to eyeball crispness across scales.
            for px in [16.0_f32, 24.0, 48.0] {
                let r = flow.cell(list, &format!("Gear {}px", px as u32), px, px);
                Icon::new(PhosphorIcon::Gear).draw(r, list);
            }

            // Tinted.
            let r = flow.cell(list, "Trash (red)", 32.0, 32.0);
            Icon::new(PhosphorIcon::Trash)
                .tint([0.90, 0.25, 0.25, 1.0])
                .draw(r, list);
        }

        // ---- Text -------------------------------------------------------
        flow.section(list, "Text");

        let r = flow.cell(list, "Plain", 190.0, 20.0);
        list.text(
            TextBlock::new("The quick brown fox", r.x, r.y)
                .with_size(16.0)
                .with_color(200, 210, 230),
        );

        let r = flow.cell(list, "Outline", 130.0, 24.0);
        list.text(
            TextBlock::new("Outlined", r.x, r.y)
                .with_size(20.0)
                .with_color(255, 255, 255)
                .with_outline(10, 12, 18, 255, 2.0),
        );

        let r = flow.cell(list, "Shadow", 120.0, 20.0);
        list.text(
            TextBlock::new("Shadowed", r.x, r.y)
                .with_size(16.0)
                .with_color(200, 220, 255)
                .with_shadow(0, 0, 0, 200, 1.0, 1.0, 1.0),
        );

        let r = flow.cell(list, "Glow", 120.0, 26.0);
        list.text(
            TextBlock::new("Glow", r.x, r.y)
                .with_size(24.0)
                .with_color(255, 255, 200)
                .with_glow(80, 200, 255, 255, 3.0),
        );

        let r = flow.cell(list, "Align right", 150.0, 20.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("Right", r.x, r.y + 2.0)
                .with_size(14.0)
                .with_color(180, 190, 210)
                .with_max_width(r.width)
                .with_align(TextAlign::Right),
        );

        let r = flow.cell(list, "Align center", 150.0, 20.0);
        list.rect_outline(r, 1.0, [0.3, 0.34, 0.42, 1.0]);
        list.text(
            TextBlock::new("Center", r.x, r.y + 2.0)
                .with_size(14.0)
                .with_color(180, 190, 210)
                .with_max_width(r.width)
                .with_align(TextAlign::Center),
        );

        // ---- Vertical centering (debug) --------------------------------
        // Visualises the per-label optical centerer (`DrawList::vcentered_text_y`).
        // The band it centres is chosen from the *text*: a label with lowercase
        // letters centres on the x-height body; an all-caps/numeric label centres
        // on the taller cap height. For each sample we draw the container box, the
        // centred text, and guide lines:
        //   • GREEN  = the box's geometric centre.
        //   • RED (two lines) = the band the centerer actually chose (its top and
        //     the baseline) — its midpoint should sit on the green line.
        //   • dim GREY = the *other*, non-chosen band top (reference only).
        // Correct centring ⇒ the red band straddles the green line: for "Hxngy"
        // the lowercase body sits centred (caps overshoot up, descenders hang
        // below); for "HX100%" the cap/digit body sits centred.
        flow.section(list, "Vertical centering (debug)");
        let samples: [&str; 2] = ["Hxngy", "HX100%"];
        for sample in samples {
            for size in [15.0f32, 24.0] {
                let box_h = (size * 1.25 + 20.0).max(36.0);
                let has_lc = sample.chars().any(|c| c.is_lowercase());
                let label = format!(
                    "{sample} ({}) @ {size:.0}px",
                    if has_lc { "x" } else { "cap" }
                );
                let r = flow.cell(list, &label, 150.0, box_h);
                // Container.
                list.rect_outline(r, 1.0, [0.30, 0.34, 0.42, 1.0]);
                // Centre this exact sample, then recover the band it chose.
                let ty = list.vcentered_text_y(r.y, r.height, size, theme.font.as_ref(), sample);
                let m = list.font_vmetrics(theme.font.as_ref());
                let baseline = ty + m.baseline_ratio * size;
                let chosen = if has_lc { m.x_ratio } else { m.cap_ratio };
                let other = if has_lc { m.cap_ratio } else { m.x_ratio };
                let chosen_top = baseline - chosen * size;
                let other_top = baseline - other * size;
                // Green: box centre.
                list.quad(
                    r.x,
                    r.y + r.height / 2.0 - 0.5,
                    r.width,
                    1.0,
                    [0.25, 1.0, 0.45, 0.7],
                );
                // Grey (reference): the non-chosen band top.
                list.quad(r.x, other_top - 0.5, r.width, 1.0, [0.55, 0.58, 0.64, 0.6]);
                // Red: the chosen band (its top + the baseline) — the centring target.
                list.quad(r.x, chosen_top - 0.5, r.width, 1.0, [1.0, 0.25, 0.25, 0.95]);
                list.quad(r.x, baseline - 0.5, r.width, 1.0, [1.0, 0.25, 0.25, 0.95]);
                list.text(
                    TextBlock::new(sample, r.x + 6.0, ty)
                        .with_size(size)
                        .with_color(228, 232, 240),
                );
            }
        }
        // CJK centres on the ideographic ink centre: ideographs overhang the em
        // square and dip below the baseline, so neither the x- nor cap-band
        // applies. Here RED = the computed visual centre, which should land on the
        // GREEN box centre with the ideograph optically centred on it. (If no CJK
        // font is installed the glyphs render as tofu and this falls back to the
        // cap-band centre.)
        for sample in ["中字", "あ漢A"] {
            for size in [15.0f32, 24.0] {
                let box_h = (size * 1.25 + 20.0).max(36.0);
                let label = format!("{sample} (cjk) @ {size:.0}px");
                let r = flow.cell(list, &label, 150.0, box_h);
                list.rect_outline(r, 1.0, [0.30, 0.34, 0.42, 1.0]);
                let ty = list.vcentered_text_y(r.y, r.height, size, theme.font.as_ref(), sample);
                let m = list.font_vmetrics(theme.font.as_ref());
                let visual_center = ty + m.visual_center_ratio(sample) * size;
                // Green: box centre.
                list.quad(
                    r.x,
                    r.y + r.height / 2.0 - 0.5,
                    r.width,
                    1.0,
                    [0.25, 1.0, 0.45, 0.7],
                );
                // Red: computed ideographic visual centre (should coincide with green).
                list.quad(
                    r.x,
                    visual_center - 0.5,
                    r.width,
                    1.0,
                    [1.0, 0.25, 0.25, 0.95],
                );
                list.text(
                    TextBlock::new(sample, r.x + 6.0, ty)
                        .with_size(size)
                        .with_color(228, 232, 240),
                );
            }
        }
        // Numeric readout of the resolved per-font ratios.
        let font_name = theme
            .font
            .as_ref()
            .map(|f| f.family().to_string())
            .unwrap_or_else(|| "default sans (Noto Sans)".to_string());
        let m = list.font_vmetrics(theme.font.as_ref());
        let r = flow.cell(list, "resolved ratios", 440.0, 36.0);
        list.text(
            TextBlock::new(
                format!(
                    "{font_name}: baseline {:.3} · x-height {:.3} · cap {:.3} · cjk-baseline {:.3} · cjk-centre {:.3}  (×font_size)",
                    m.baseline_ratio, m.x_ratio, m.cap_ratio, m.cjk_baseline_ratio, m.cjk_center_ratio
                ),
                r.x,
                r.y + 9.0,
            )
            .with_size(13.0)
            .with_color(200, 210, 230),
        );

        // ---- Fonts ------------------------------------------------------
        // The bundled Noto Sans family (registered by `shared_font_system`)
        // resolves the default sans-serif and provides real bold/italic faces.
        flow.section(list, "Fonts");

        let r = flow.cell(list, "Regular", 130.0, 24.0);
        list.text(
            TextBlock::new("Regular", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235),
        );

        let r = flow.cell(list, "Bold", 130.0, 24.0);
        list.text(
            TextBlock::new("Bold", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .bold(),
        );

        let r = flow.cell(list, "Italic", 130.0, 24.0);
        list.text(
            TextBlock::new("Italic", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .italic(),
        );

        let r = flow.cell(list, "Bold Italic", 140.0, 24.0);
        list.text(
            TextBlock::new("Bold Italic", r.x, r.y)
                .with_size(22.0)
                .with_color(220, 225, 235)
                .bold()
                .italic(),
        );

        // ---- Span-coloured text ----------------------------------------
        flow.section(list, "Span colour + underline");

        let r = flow.cell(list, "Colour spans", 200.0, 24.0);
        list.text(
            TextBlock::new("", r.x, r.y)
                .with_size(20.0)
                .with_color(255, 255, 255)
                .with_spans(vec![
                    TextSpan {
                        text: "Red".into(),
                        color: Some([1.0, 0.2, 0.2, 1.0]),
                        underline: None,
                    },
                    TextSpan {
                        text: " · ".into(),
                        color: Some([0.8, 0.8, 0.8, 1.0]),
                        underline: None,
                    },
                    TextSpan {
                        text: "Green".into(),
                        color: Some([0.2, 1.0, 0.4, 1.0]),
                        underline: None,
                    },
                    TextSpan {
                        text: " · ".into(),
                        color: Some([0.8, 0.8, 0.8, 1.0]),
                        underline: None,
                    },
                    TextSpan {
                        text: "Blue".into(),
                        color: Some([0.3, 0.6, 1.0, 1.0]),
                        underline: None,
                    },
                ]),
        );

        let r = flow.cell(list, "Underline", 200.0, 28.0);
        list.text(
            TextBlock::new("", r.x, r.y)
                .with_size(20.0)
                .with_color(220, 225, 235)
                .with_spans(vec![
                    TextSpan {
                        text: "normal ".into(),
                        color: None,
                        underline: None,
                    },
                    TextSpan {
                        text: "underlined".into(),
                        color: Some([1.0, 0.9, 0.3, 1.0]),
                        underline: Some([1.0, 0.8, 0.0, 1.0]),
                    },
                    TextSpan {
                        text: " end".into(),
                        color: None,
                        underline: None,
                    },
                ]),
        );

        // ---- Interactive verbs (UiContext) ------------------------------
        // The crate-side stateful façade: each verb places + localizes the raw
        // widget and auto-advances a vertical cursor. Rendered at rest (the
        // static InputState isn't interacting), so this just eyeballs layout +
        // crispness of the verb stack.
        flow.section(list, "Interactive verbs (UiContext)");
        {
            let r = flow.cell(list, "Stacked verbs", 200.0, 168.0);
            let mut vstate = UiState::new();
            // Static render: dt = 0 freezes the animation clock so the verbs draw
            // their resolved (settled) colors, keeping the PNG deterministic.
            vstate.begin_frame(&input, &theme, 0.0);
            let mut buf = String::from("editable");
            // Scope the `ui.translate` to this block: `UiContext::translate`
            // mutates the shared list's transform-stack top in place and is not
            // restored on drop, so without a push/pop bracket the translate
            // leaks and shifts every later base-layer cell (the whole Widgets
            // section) off-position.
            list.push_transform();
            {
                let mut ui = UiContext::interactive(list, &input, &mut vstate, &theme);
                ui.translate(r.x, r.y);
                ui.text("text() label");
                ui.text_button("text_button()", Some(200.0), None);
                let _ = ui.slider(0, 0.6, 0.0, 1.0, Some(200.0));
                let _ = ui.checkbox("checkbox()", true);
                let _ = ui.text_input(1, &mut buf, "type…", Some(200.0));
            }
            // The verbs auto-advanced the transform cursor down from `r.y`; the
            // delta is the stack's true painted height. Reserve it so the cell's
            // 168px nominal height doesn't let the next section overlap (the
            // stack is taller than that). Read before `pop_transform` restores it.
            let stack_h = list.current_transform().ty - r.y;
            list.pop_transform();
            flow.reserve(stack_h);
            vstate.end_frame();
        }

        // ---- Widgets ----------------------------------------------------
        flow.section(list, "Widgets");

        let r = flow.cell(list, "Button", 100.0, 32.0);
        Button::draw_at(
            "Button",
            r,
            true,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        let r = flow.cell(list, "Button (bare)", 100.0, 32.0);
        Button::new("Bare")
            .bare()
            .draw(r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Button (disabled)", 110.0, 32.0);
        Button::draw_at(
            "Disabled",
            r,
            false,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        // Keyboard-focused button: seed a local focus owner so the focus ring is
        // visible in the PNG without disturbing the shared focus state.
        let r = flow.cell(list, "Button (focused)", 110.0, 32.0);
        {
            const FOCUSED_BTN: u64 = 300;
            let btn_idle = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let mut btn_focus = FocusState::new();
            btn_focus.focus(FOCUSED_BTN);
            Button::new("Focused").focusable(FOCUSED_BTN).draw(
                r,
                &mut DrawContext::new(list, &mut btn_focus, &theme, &btn_idle, W as f32, 600.0),
            );
        }

        let cb = Checkbox::new();
        let r = flow.cell(list, "Checkbox", 120.0, 20.0);
        cb.draw(false, "Off", r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Checkbox (checked)", 120.0, 20.0);
        cb.draw(true, "On", r, &mut ctx(list, &mut focus, &theme, &input));

        let radio_opts = ["Low", "Medium", "High"];
        let r = flow.cell(list, "Radio group", 120.0, 76.0);
        RadioGroup::new(&radio_opts).draw(1, r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Radio (horizontal)", 260.0, 24.0);
        RadioGroup::new(&radio_opts)
            .horizontal()
            .draw(0, r, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Progress bar", 150.0, 20.0);
        ProgressBar::new(0.65).draw(r, list, &StyleResolver::new(&theme));

        let r = flow.cell(list, "Slider", 160.0, 24.0);
        let mut capture = DragCapture::default();
        Slider::new(0.0, 100.0).draw(
            40.0,
            0,
            &mut capture,
            r,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        // Drag handle / window-mover: a labelled title bar and a bare grip
        // handle. Static (idle) here — the live delta comes from a DragTracker.
        let r = flow.cell(list, "Drag handle (title bar)", 200.0, 24.0);
        let mut dh_cap = DragCapture::default();
        DragHandle::new().with_label("Inspector").draw(
            10,
            &mut dh_cap,
            r,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        let r = flow.cell(list, "Drag handle (grip)", 64.0, 24.0);
        DragHandle::new().draw(
            11,
            &mut dh_cap,
            r,
            &mut ctx(list, &mut focus, &theme, &input),
        );

        let r = flow.cell(list, "Tabs", 240.0, 30.0);
        Tabs::new(&["Tab A", "Tab B", "Tab C"]).draw(
            r,
            0,
            list,
            &StyleResolver::new(&theme),
            &input,
            None,
        );

        let r = flow.cell(list, "Text input", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_value("Hello, wgpu-gameui!")
            .draw(0, &mut ctx(list, &mut focus, &theme, &input));

        let r = flow.cell(list, "Text input (empty)", 200.0, 28.0);
        TextInput::new(r.x, r.y, r.width, r.height)
            .with_placeholder("Placeholder...")
            .draw(1, &mut ctx(list, &mut focus, &theme, &input));

        // Text input mid-IME-composition: focused, with a non-empty preedit
        // spliced into the value at the caret and rendered underlined. Uses a
        // local focus+input so it doesn't disturb the shared focus owner above.
        let r = flow.cell(list, "Text input (composing)", 200.0, 28.0);
        {
            const COMPOSE_ID: u64 = 200;
            let mut compose_input = InputState::default();
            compose_input.preedit = "nihongo".to_string();
            let mut compose_focus = FocusState::new();
            compose_focus.focus(COMPOSE_ID);
            compose_focus.begin_frame(&compose_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height).with_value("ab cd");
            field.cursor_pos = 3; // caret after "ab ", before "cd"
            field.draw(
                COMPOSE_ID,
                &mut DrawContext::new(
                    list,
                    &mut compose_focus,
                    &theme,
                    &compose_input,
                    W as f32,
                    600.0,
                ),
            );
        }

        // Multi-line text area: focused, with two hard newlines and one line long
        // enough to wrap at the field width. Uses a local focus+input so it
        // doesn't disturb the shared focus owner above.
        let r = flow.cell(list, "Text area (multiline)", 200.0, 86.0);
        {
            const AREA_ID: u64 = 201;
            let area_input = InputState::default();
            let mut area_focus = FocusState::new();
            area_focus.focus(AREA_ID);
            area_focus.begin_frame(&area_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height)
                .with_multiline(true)
                .with_value("line one\nsecond line\nthis third line is long enough to wrap");
            field.cursor_pos = field.value.len();
            field.draw(
                AREA_ID,
                &mut DrawContext::new(list, &mut area_focus, &theme, &area_input, W as f32, 600.0),
            );
        }

        // Number input / spin box: a focused float field showing the +/- step
        // buttons in the right column and an editable value.
        let r = flow.cell(list, "Number input", 140.0, 28.0);
        {
            const NUM_ID: u64 = 202;
            let num_input = InputState::default();
            let mut num_focus = FocusState::new();
            num_focus.focus(NUM_ID);
            num_focus.begin_frame(&num_input);
            let mut field = TextInput::new(r.x, r.y, r.width, r.height);
            NumberInput::new()
                .with_range(0.0, 100.0)
                .with_step(1.0)
                .with_decimals(1)
                .draw(
                    42.5,
                    NUM_ID,
                    &mut field,
                    r,
                    &mut DrawContext::new(
                        list,
                        &mut num_focus,
                        &theme,
                        &num_input,
                        W as f32,
                        600.0,
                    ),
                );
        }

        // Tree view (outliner): a seeded hierarchy — an expanded branch with
        // indented children (one selected), a collapsed branch, and a root leaf.
        // Each row carries a leading "visibility" icon plus trailing
        // rename/delete icons (their own hit targets), demonstrating the
        // scene/layer-outliner shape. Idle input, so nothing toggles.
        let r = flow.cell(list, "Tree view (outliner)", 200.0, 110.0);
        {
            const VIS: u32 = 1;
            const RENAME: u32 = 2;
            const DEL: u32 = 3;
            let mut tree = TreeState::new();
            tree.set_expanded(1, true); // "Materials" expanded
            tree.set_expanded(4, false); // "Foliage" collapsed
            tree.select(3); // "Metal" selected
            let idle = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let leading = [TreeAction::sprite(VIS, ball)];
            let trailing = [
                TreeAction::sprite(RENAME, board),
                TreeAction::sprite(DEL, suitcase),
            ];
            let rows: [(u64, &str, bool, usize); 5] = [
                (1, "Materials", false, 0),
                (2, "Wood", true, 1),
                (3, "Metal", true, 1),
                (4, "Foliage", false, 0),
                (5, "Stone", true, 0),
            ];
            for (i, (id, label, leaf, depth)) in rows.iter().enumerate() {
                let row = Rect::new(r.x, r.y + i as f32 * 21.0, r.width, 20.0);
                let mut tctx = DrawContext::new(list, &mut focus, &theme, &idle, W as f32, 600.0);
                TreeNode::new(label)
                    .with_leaf(*leaf)
                    .with_depth(*depth)
                    .with_leading(&leading)
                    .with_trailing(&trailing)
                    .draw(*id, row, &mut tree, &mut tctx);
            }
        }

        // Dropdown, seeded open: the floating list (drawn after the base scope)
        // renders above whatever cells sit below it.
        let r = flow.cell(list, "Dropdown (open)", 160.0, 28.0);
        dropdowns.open_for_test(DROPDOWN_ID, r, &DROPDOWN_ITEMS, 2);
        let dropdown = Dropdown::new(&DROPDOWN_ITEMS, 2);
        // The open list is an overlay (drawn later into a popup layer), so the
        // cell only nominally reserves the 28px button. Reserve its full open
        // footprint too, so the floating list doesn't paint over the rows below.
        let menu = dropdown.open_list_rect(r);
        flow.reserve(menu.y + menu.height - r.y);
        dropdown.draw(
            DROPDOWN_ID,
            r,
            &mut dropdowns,
            &mut DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0),
        );

        let r = flow.cell(list, "Scroll view", 180.0, 100.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        let mut scroll_state = ScrollState::default();
        scroll_state.content_size = [160.0, 300.0];
        ScrollView::new(r).vertical_only().draw(
            &mut scroll_state,
            list,
            &StyleResolver::new(&theme),
            &mut input,
            |list, vp| {
                for i in 0..12usize {
                    let y = vp.y + i as f32 * 22.0;
                    let bg = if i % 2 == 0 {
                        [0.16, 0.18, 0.24, 1.0]
                    } else {
                        [0.10, 0.12, 0.18, 1.0]
                    };
                    // `vp` already excludes the scrollbar gutter, so fill it
                    // edge-to-edge; the row only pads its own text.
                    list.quad(vp.x, y + 2.0, vp.width, 18.0, bg);
                    list.text(
                        TextBlock::new(&format!("Item #{:02}", i), vp.x + 8.0, y + 3.0)
                            .with_size(12.0)
                            .with_color(180, 190, 210),
                    );
                }
            },
        );

        let columns = &[
            TableColumn::new("Name", ColumnWidth::Fixed(100.0)),
            TableColumn::new("Score", ColumnWidth::Fixed(60.0)),
            TableColumn::new("Status", ColumnWidth::Flex(1.0)),
        ];
        let rows = vec![
            vec![
                TableCell::new("Alice"),
                TableCell::new("95"),
                TableCell::new("Pass"),
            ],
            vec![
                TableCell::new("Bob"),
                TableCell::new("72"),
                TableCell::new("Pass"),
            ],
            vec![
                TableCell::new("Charlie"),
                TableCell::new("48"),
                TableCell::new("Fail"),
            ],
        ];
        let r = flow.cell(list, "Table", 270.0, 88.0);
        list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
        Table::new(columns).draw(
            r,
            &rows,
            &mut ScrollState::default(),
            list,
            &StyleResolver::new(&theme),
            &mut input,
        );

        let r = flow.cell(list, "Image button", 40.0, 40.0);
        ImageButton::sprite(duck)
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &StyleResolver::new(&theme), &input);

        let r = flow.cell(list, "Image button (bare)", 40.0, 40.0);
        ImageButton::sprite(board)
            .bare()
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &StyleResolver::new(&theme), &input);

        let r = flow.cell(list, "Image button (disabled)", 40.0, 40.0);
        ImageButton::sprite(suitcase)
            .enabled(false)
            .fit(ImageFit::Contain)
            .natural_size(48.0, 48.0)
            .draw(r, list, &StyleResolver::new(&theme), &input);

        // ---- Lists / Grids (virtualized) --------------------------------
        flow.section(list, "Lists / Grids");

        // (a) Vertical list: 12 rows, one selected + one hovered (seeded by
        // pointing the idle mouse at row 4 so the hover background shows).
        {
            let items: [&str; 12] = [
                "Sword", "Shield", "Potion", "Bow", "Arrow", "Helmet", "Gauntlet", "Boots", "Ring",
                "Amulet", "Scroll", "Torch",
            ];
            let r = flow.cell(list, "List (selectable)", 150.0, 150.0);
            list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
            let hover = InputState {
                mouse_x: r.x + 20.0,
                mouse_y: r.y + 22.0 * 4.0 + 8.0,
                ..InputState::default()
            };
            let mut state = ListState::new();
            state.select_one(1); // "Shield" selected
            let mut hover_in = hover;
            List::new()
                .with_item_height(22.0)
                .with_zebra(true)
                .selection(SelectionMode::Single)
                .draw(
                    r,
                    items.len(),
                    &mut state,
                    list,
                    &StyleResolver::new(&theme),
                    &mut hover_in,
                    |list, cell, it: ListItem| {
                        // Debug: outline the cell rect handed to the closure, so
                        // the item's content padding is visible.
                        list.rect_outline(cell, 1.0, [1.0, 0.25, 0.8, 0.9]);
                        let c = if it.selected {
                            (20, 24, 34)
                        } else {
                            (200, 210, 230)
                        };
                        list.text(
                            TextBlock::new(items[it.index], cell.x + 8.0, cell.y + 4.0)
                                .with_size(13.0)
                                .with_color(c.0, c.1, c.2),
                        );
                    },
                );
        }

        // (b) Grid: 4 columns of colored tiles with an index label.
        {
            let r = flow.cell(list, "Grid (4 cols)", 150.0, 150.0);
            list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
            let mut state = ListState::new();
            state.select_one(5);
            let mut idle_in = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            List::new()
                .with_item_height(32.0)
                .columns(4)
                .with_gap(6.0, 6.0)
                .selection(SelectionMode::Multi)
                .draw(
                    r,
                    24,
                    &mut state,
                    list,
                    &StyleResolver::new(&theme),
                    &mut idle_in,
                    |list, cell, it: ListItem| {
                        // Debug: outline the cell rect handed to the closure.
                        list.rect_outline(cell, 1.0, [1.0, 0.25, 0.8, 0.9]);
                        // Tile fill: a hue ramp so the grid reads as distinct cells.
                        let t = it.index as f32 / 24.0;
                        let fill = if it.selected {
                            [0.95, 0.85, 0.30, 1.0]
                        } else {
                            [0.20 + 0.5 * t, 0.30, 0.55 - 0.3 * t, 1.0]
                        };
                        list.rounded_rect(
                            Rect::new(
                                cell.x + 2.0,
                                cell.y + 2.0,
                                cell.width - 4.0,
                                cell.height - 4.0,
                            ),
                            3.0,
                            fill,
                        );
                        list.text(
                            TextBlock::new(&format!("{}", it.index), cell.x + 6.0, cell.y + 9.0)
                                .with_size(12.0)
                                .with_color(240, 245, 255),
                        );
                    },
                );
        }

        // (c) Tall list in a short cell: shows the scrollbar + virtualization
        // (1000 items, scrolled partway down).
        {
            let r = flow.cell(list, "Virtualized (1000)", 150.0, 110.0);
            list.rounded_rect(r, 4.0, [0.06, 0.07, 0.10, 1.0]);
            let mut state = ListState::new();
            state.scroll.offset[1] = 420.0; // scrolled partway
            let mut idle_in = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            List::new().with_item_height(20.0).draw(
                r,
                1000,
                &mut state,
                list,
                &StyleResolver::new(&theme),
                &mut idle_in,
                |list, cell, it: ListItem| {
                    // Debug: outline the cell rect handed to the closure. The
                    // cell already excludes the scrollbar gutter (ScrollView
                    // reserves it), so the item fills the cell edge-to-edge and
                    // only pads its *own* text.
                    list.rect_outline(cell, 1.0, [1.0, 0.25, 0.8, 0.9]);
                    let bg = if it.index % 2 == 0 {
                        [0.13, 0.15, 0.20, 1.0]
                    } else {
                        [0.09, 0.11, 0.16, 1.0]
                    };
                    list.quad(cell.x, cell.y, cell.width, cell.height, bg);
                    list.text(
                        TextBlock::new(
                            &format!("Row #{:04}", it.index),
                            cell.x + 8.0,
                            cell.y + 3.0,
                        )
                        .with_size(12.0)
                        .with_color(180, 190, 210),
                    );
                },
            );
        }

        // ---- Instanced chrome (SDF rounded-rect) ------------------------
        // Every `Button` already routes its background+border through the
        // instanced `chrome_rect` path; this section makes the batching
        // explicit (a strip of same-shape buttons collapses to one base mesh +
        // N instances) and shows the rotated-transform fallback still renders.
        flow.section(list, "Instanced chrome");

        for i in 0..6 {
            let r = flow.cell(list, "", 70.0, 30.0);
            Button::new(format!("#{i}")).draw(r, &mut ctx(list, &mut focus, &theme, &input));
        }

        // Rotated chrome: `chrome_rect` can't express a rotation as a single
        // axis-aligned instance, so it falls back to immediate tessellation.
        let r = flow.cell(list, "Rotated (fallback)", 80.0, 40.0);
        list.push_transform();
        list.translate(r.x + r.width / 2.0, r.y + r.height / 2.0);
        list.rotate(0.18);
        list.chrome_rect(
            Rect::new(-r.width / 2.0, -r.height / 2.0, r.width, r.height),
            8.0,
            2.0,
            [0.30, 0.55, 0.35, 1.0],
            [0.80, 0.90, 0.80, 1.0],
        );
        list.pop_transform();

        // ---- Hit zone (draw-free sensor) --------------------------------
        // `HitZone` draws NOTHING — it only senses pointer interaction over a
        // rect (Teardown's UiMakeInteractive), for sensors over things the UI
        // didn't draw (3D viewports, world-projected regions). The gallery
        // can't show "nothing", so each cell paints its own outline + a caption
        // reporting the state `HitZone::test` returns for a synthetic pointer.
        flow.section(list, "Hit zone (sensor)");

        // Idle: pointer parked far away → not hovered.
        let r = flow.cell(list, "Idle (no pointer)", 150.0, 44.0);
        {
            let away = InputState {
                mouse_x: -1.0,
                mouse_y: -1.0,
                ..InputState::default()
            };
            let out = HitZone::new().test(r, &away);
            list.rounded_rect_outline(r, 4.0, 1.5, [0.40, 0.45, 0.55, 1.0]);
            list.text(
                TextBlock::new(
                    if out.hovered { "hovered" } else { "idle" },
                    r.x + 10.0,
                    r.y + 14.0,
                )
                .with_size(12.0)
                .with_color(150, 160, 180),
            );
        }

        // Hovered + clicked: synthetic pointer at the cell centre with a click.
        let r = flow.cell(list, "Hovered + click", 150.0, 44.0);
        {
            let over = InputState {
                mouse_x: r.x + r.width / 2.0,
                mouse_y: r.y + r.height / 2.0,
                mouse_down: true,
                mouse_clicked: true,
                ..InputState::default()
            };
            let out = HitZone::new().test(r, &over);
            // Highlight to reflect the sensed hover (the widget itself draws
            // none of this — the gallery does, from the returned state).
            let glow = if out.hovered {
                [0.20, 0.55, 0.95, 0.18]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            };
            list.rounded_rect(r, 4.0, glow);
            list.rounded_rect_outline(r, 4.0, 1.5, [0.35, 0.65, 1.0, 1.0]);
            let caption = if out.clicked {
                "hovered + clicked"
            } else if out.hovered {
                "hovered"
            } else {
                "idle"
            };
            list.text(
                TextBlock::new(caption, r.x + 10.0, r.y + 14.0)
                    .with_size(12.0)
                    .with_color(200, 215, 240),
            );
        }

        // --- Styling / overrides -------------------------------------------
        // Per-widget restyling with NO theme clone: a scoped `StyleOverlay`
        // layered over the theme via `DrawContext::with_style`, plus a custom
        // (mod-defined) key resolved by name.
        flow.section(list, "Styling / overrides");

        // Baseline button — straight theme colors.
        let r = flow.cell(list, "Button (theme)", 120.0, 32.0);
        Button::new("Normal").draw(r, &mut ctx(list, &mut focus, &theme, &input));

        // Same widget under an overlay — recolored fill/border/text only.
        let mut overlay = StyleOverlay::new();
        overlay
            .set_color(StyleKey::Button, [0.45, 0.12, 0.55, 1.0])
            .set_color(StyleKey::ButtonBorder, [0.85, 0.55, 0.95, 1.0])
            .set_color(StyleKey::Text, [1.0, 0.92, 1.0, 1.0]);
        let r = flow.cell(list, "Button (overlay)", 120.0, 32.0);
        {
            let mut octx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
                .with_style(&overlay);
            Button::new("Restyled").draw(r, &mut octx);
        }

        // Custom key: a mod-defined style resolved through the overlay (a custom
        // widget can carry its own style with zero core changes). Swatch + name.
        let mut custom = StyleOverlay::new();
        let glow_key = StyleKey::custom("mywidget.glow");
        custom.set_color(glow_key, [0.20, 0.85, 0.65, 1.0]);
        let r = flow.cell(list, "Custom key", 170.0, 32.0);
        let resolver = StyleResolver::with_overlay(&theme, &custom);
        let c = resolver.color_or(glow_key, [1.0, 0.0, 1.0, 1.0]);
        list.rounded_rect(Rect::new(r.x, r.y, 28.0, 28.0), 6.0, c);
        list.text(
            TextBlock::new("mywidget.glow", r.x + 36.0, r.y + 8.0)
                .with_size(12.0)
                .with_color(200, 210, 230),
        );

        // --- Layout primitives (weighted + flow) ---------------------------
        // The declarative layout engine (`wgpu_gameui::layout`), not widgets:
        // a weighted `HStack` Fill split and the wrapping `Flow` grid. Each
        // computed `Rect` is painted as a plain rounded rect so the split ratios
        // and the row-wrapping are eyeballable.
        flow.section(list, "Layout: weighted HStack + Flow grid");

        // Weighted HStack — remaining width split 2:1:1 across three Fill cells.
        {
            let r = flow.cell(list, "HStack weight 2:1:1", 300.0, 36.0);
            let split = HStack::new(6.0)
                .child_fill(0.0)
                .weight(2.0)
                .child_fill(0.0)
                .weight(1.0)
                .child_fill(0.0)
                .weight(1.0);
            let res = split.layout(r);
            let colors = [
                [0.30, 0.50, 0.90, 1.0],
                [0.30, 0.75, 0.55, 1.0],
                [0.85, 0.55, 0.30, 1.0],
            ];
            for (i, c) in colors.iter().enumerate() {
                list.rounded_rect(res.get(i + 1), 4.0, *c);
            }
        }

        // Flow grid — nine uniform 40px tiles wrapping within a fixed width.
        {
            let grid_w = 200.0;
            let mut grid = LayoutFlow::new(8.0);
            for _ in 0..9 {
                grid = grid.item(40.0, 40.0);
            }
            let grid_h = grid.measure_height(grid_w);
            let r = flow.cell(list, "Flow grid (wraps)", grid_w, grid_h);
            let res = grid.layout(r);
            for (i, rc) in res.children().enumerate() {
                let t = i as f32 / 8.0;
                list.rounded_rect(rc, 6.0, [0.25 + 0.5 * t, 0.45, 0.85 - 0.4 * t, 1.0]);
            }
        }

        // Main-axis justification (justify-content) — the same three fixed-size
        // cells distributed six ways across a fixed-width track, so the spacing
        // policies are eyeballable stacked vertically.
        flow.section(list, "Justify (main-axis distribution)");
        for (label, mode) in [
            ("Start", MainAlign::Start),
            ("Center", MainAlign::Center),
            ("End", MainAlign::End),
            ("SpaceBetween", MainAlign::SpaceBetween),
            ("SpaceAround", MainAlign::SpaceAround),
            ("SpaceEvenly", MainAlign::SpaceEvenly),
        ] {
            let track_w = 300.0;
            let r = flow.cell(list, label, track_w, 28.0);
            // Faint track backing so empty space reads as "the container".
            list.rounded_rect(r, 4.0, [0.16, 0.16, 0.20, 1.0]);
            let row = HStack::new(0.0)
                .justify(mode)
                .child(44.0, 24.0)
                .child(44.0, 24.0)
                .child(44.0, 24.0);
            let res = row.layout(r);
            for rc in res.children() {
                list.rounded_rect(rc, 4.0, [0.30, 0.55, 0.90, 1.0]);
            }
        }

        // --- Separators / dividers -----------------------------------------
        // Thin rules, centered in their cell. Defaults pull thickness from the
        // theme border width and color from the panel-border; the third row
        // overrides both. The vertical demo splits a cell into two columns.
        flow.section(list, "Separator / divider");
        {
            let style = StyleResolver::new(&theme);

            // Plain horizontal rule (theme defaults), centered in a tall cell.
            let r = flow.cell(list, "horizontal", 200.0, 20.0);
            Separator::horizontal().draw(r, list, &style);

            // Inset horizontal rule between two faux text lines.
            let r = flow.cell(list, "inset 16px", 200.0, 40.0);
            list.text(TextBlock::new("above", r.x, r.y).with_size(13.0));
            Separator::horizontal()
                .with_inset(16.0)
                .draw(Rect::new(r.x, r.y + 18.0, r.width, 4.0), list, &style);
            list.text(TextBlock::new("below", r.x, r.y + 24.0).with_size(13.0));

            // Thick accent rule (overridden thickness + color).
            let r = flow.cell(list, "thick accent", 200.0, 20.0);
            Separator::horizontal()
                .with_thickness(4.0)
                .with_color(theme.accent)
                .draw(r, list, &style);

            // Vertical divider splitting a cell into two columns.
            let r = flow.cell(list, "vertical", 120.0, 48.0);
            list.text(TextBlock::new("L", r.x + 16.0, r.y + 16.0).with_size(13.0));
            Separator::vertical().with_inset(6.0).draw(
                Rect::new(r.x + r.width * 0.5 - 2.0, r.y, 4.0, r.height),
                list,
                &style,
            );
            list.text(TextBlock::new("R", r.x + r.width - 28.0, r.y + 16.0).with_size(13.0));
        }

        // --- Color picker --------------------------------------------------
        // SV square (white→hue across, →black down) + vertical hue spectrum,
        // optionally an alpha bar (checkerboard under an opaque→transparent
        // fade). Cursors sit at the fixed sample colors below.
        flow.section(list, "Color picker");
        {
            let mut cap = DragCapture::new();

            // HSV only — a warm orange.
            let r = flow.cell(list, "HSV (no alpha)", 220.0, 120.0);
            {
                let mut c = ctx(list, &mut focus, &theme, &input);
                ColorPicker::new().draw(Hsva::opaque(28.0, 0.85, 0.95), 900, &mut cap, r, &mut c);
            }

            // HSVA — a half-transparent teal, showing the alpha bar.
            let r = flow.cell(list, "HSVA (alpha bar)", 248.0, 120.0);
            {
                let mut c = ctx(list, &mut focus, &theme, &input);
                ColorPicker::new().with_alpha(true).draw(
                    Hsva::new(175.0, 0.7, 0.8, 0.5),
                    901,
                    &mut cap,
                    r,
                    &mut c,
                );
            }
        }

        // --- Hover animation (easing) --------------------------------------
        // The animation system eases a widget's color from its idle value toward
        // its hover value over `theme.animation_duration`. A static PNG has no
        // time axis, so we sample the *same* ease-out curve at five linear points
        // t ∈ {0, .25, .5, .75, 1} and paint a Button at each step.
        //
        // The endpoints here are exaggerated — idle slate (`button`) → bright
        // `accent` — *on purpose*: the real default hover delta (`button` →
        // `button_hover`) is only ~0.04/channel and reads as flat gray at this
        // size. With a high-contrast pair the ease-out shape is legible: the
        // steps bunch toward the bright end (fast start, slow finish). Drawn via
        // the public `ease`/`lerp_color` through a per-button `StyleOverlay`.
        flow.section(list, "Hover animation (ease-out curve)");
        for &t in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let eased = ease(Easing::EaseOut, t);
            let fill = lerp_color(theme.button, theme.accent, eased);
            let mut ramp = StyleOverlay::new();
            ramp.set_color(StyleKey::Button, fill);
            let label = format!("t={t:.2}");
            let r = flow.cell(list, &label, 90.0, 32.0);
            let mut rctx = DrawContext::new(list, &mut focus, &theme, &input, W as f32, 600.0)
                .with_style(&ramp);
            Button::new(&label).draw(r, &mut rctx);
        }

        // Tooltip target last: its popup floats down-and-right into the empty
        // headroom below, overlapping no other widget.
        let r = flow.cell(list, "Tooltip target", 120.0, 24.0);
        list.rounded_rect(r, 4.0, [0.25, 0.30, 0.40, 1.0]);
        list.text(
            TextBlock::new("Hover me", r.x + 8.0, r.y + 5.0)
                .with_size(12.0)
                .with_color(200, 210, 230),
        );
        tooltip_rect = r;

        // Leave headroom below the last row for the tooltip popup.
        content_bottom = flow.bottom() + 70.0;
    }

    // Size the target to the laid-out content first, so the tooltip layer
    // knows the real screen height (it flips the popup up/left near the edges).
    let h = (content_bottom.ceil() as u32).max(64);

    // Floating dropdown list (Popup layer above the base content).
    {
        let popup = dropdowns.push_open_layer(&mut layers);
        dropdowns.draw_open_layer(&mut layers, popup, &StyleResolver::new(&theme), &InputState::default());
    }

    // Tooltip layer, hovering the reserved target.
    {
        let mut tooltip = TooltipLayer::new();
        tooltip.register(tooltip_rect, TooltipContent::text("This is a tooltip!"));
        let mut tip_input = InputState::default();
        tip_input.mouse_x = tooltip_rect.x + tooltip_rect.width / 2.0;
        tip_input.mouse_y = tooltip_rect.y + tooltip_rect.height / 2.0;
        tooltip.tick(999.0, &tip_input);
        tooltip.draw_into_layers(&mut layers, &tip_input, &StyleResolver::new(&theme), W as f32, h as f32);
    }

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gallery target"),
        size: wgpu::Extent3d {
            width: W,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // bytes_per_row must be 256-aligned for wgpu copy.
    let row_stride = W * 4;
    let bytes_per_row = (row_stride + 255) & !255;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("encoder"),
    });
    {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.06,
                        b: 0.08,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    ui.render_layers(&device, &queue, &mut encoder, &view, (W, h), 1.0, &layers);

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: h,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    // De-pad: the GPU buffer rows are 256-aligned (`bytes_per_row`), but a
    // tightly-packed RGBA image expects `row_stride` (W*4) per row. Copy each
    // row's real bytes, dropping the alignment padding — otherwise every row
    // drifts by the padding amount and the image shears diagonally.
    let row_stride = (W * 4) as usize;
    let bpr = bytes_per_row as usize;
    let mut pixels = Vec::with_capacity(row_stride * h as usize);
    for row in 0..h as usize {
        let start = row * bpr;
        pixels.extend_from_slice(&data[start..start + row_stride]);
    }

    std::fs::create_dir_all("test_output").unwrap();
    let img = image::RgbaImage::from_raw(W, h, pixels).expect("image from raw");
    img.save("test_output/widget_gallery.png")
        .expect("save png");
    eprintln!("wrote test_output/widget_gallery.png ({W}x{h})");

    // Sanity: at least some pixels are not the clear color.
    let clear = [13u8, 15, 20];
    let drew = img.pixels().any(|p| {
        let d = (p.0[0] as i32 - clear[0] as i32).abs()
            + (p.0[1] as i32 - clear[1] as i32).abs()
            + (p.0[2] as i32 - clear[2] as i32).abs();
        d > 30
    });
    assert!(
        drew,
        "no widget pixels rendered — pipeline produced an empty frame"
    );
}
