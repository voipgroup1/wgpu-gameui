//! Headless tests: pure geometry, the activation state machine, pointer
//! integration, and the layer/hit-region plumbing. No GPU needed.

use crate::layout::Rect;
use crate::{
    DrawContext, DrawList, FocusState, InputState, InteractionScene, LayerStack, Menu, MenuBar,
    MenuBarOutput, MenuBarState, MenuDrawEnv, MenuItem, Theme,
};

use super::model::{
    AccelPlatform, Accelerator, Key, MenuTrigger, Modifiers, SubmenuSide, bar_label_id,
    blocker_region_id, column_blocker_id, row_id,
};
use super::placement::{blocker_regions, place_popup};

const BAR: u64 = 0x5EED;
const W: f32 = 800.0;
const H: f32 = 600.0;

const NEW_ID: u64 = 101;
const QUIT_ID: u64 = 102;
const COPY_ID: u64 = 201;

const RECENT: &[MenuItem<'static>] = &[MenuItem::new("Recent file")];

const FILE_ITEMS: &[MenuItem<'static>] = &[
    MenuItem::new("New")
        .id(NEW_ID)
        .accel(Accelerator::primary(Key::Char('N'))),
    MenuItem::new("Open…").shortcut("Ctrl+O"),
    MenuItem::separator(),
    MenuItem::new("Bold").checked(true),
    MenuItem::new("Locked").enabled(false),
    MenuItem::new("Open Recent").with_children(RECENT),
    MenuItem::new("Quit")
        .id(QUIT_ID)
        .accel(Accelerator::primary_shift(Key::Char('Q'))),
];

const EDIT_ITEMS: &[MenuItem<'static>] = &[
    MenuItem::new("Copy").id(COPY_ID),
    MenuItem::new("Paste").id(202),
];

const VIEW_ITEMS: &[MenuItem<'static>] = &[MenuItem::new("Zoom")];

const MENUS: &[Menu<'static>] = &[
    Menu::new("File").with_items(FILE_ITEMS),
    Menu::new("Edit").with_items(EDIT_ITEMS),
    Menu::new("View").enabled(false).with_items(VIEW_ITEMS),
];

fn bar() -> MenuBar<'static> {
    MenuBar::new(BAR, MENUS)
}

fn strip() -> Rect {
    Rect::new(8.0, 4.0, 400.0, 26.0)
}

fn viewport() -> Rect {
    Rect::new(0.0, 0.0, W, H)
}

/// A whole bar-only frame: no popup layers, no scene teardown. Enough for the
/// state machine, which resolves at the bar level.
///
/// The frame-top sequence runs first, because that is where the arming trigger,
/// the intent capture and the claiming happen — a test can only observe any of
/// the three through a frame.
fn bar_frame(
    state: &mut MenuBarState,
    input: &mut InputState,
    focus: &mut FocusState,
) -> MenuBarOutput {
    state.begin_frame(input);
    let theme = Theme::default();
    let mut list = DrawList::new();
    let mut scene = InteractionScene::new();
    let mut ctx =
        DrawContext::new(&mut list, focus, &theme, input, W, H).with_interactions(&mut scene);
    bar().draw(strip(), state, &mut ctx)
}

/// One full frame through the real call order, over persistent state.
struct Rig {
    state: MenuBarState,
    scene: InteractionScene,
    theme: Theme,
    input: InputState,
    focus: FocusState,
    layers: LayerStack,
}

impl Rig {
    fn new() -> Self {
        Self {
            state: MenuBarState::new(),
            scene: InteractionScene::new(),
            theme: Theme::default(),
            input: InputState::default(),
            focus: FocusState::new(),
            layers: LayerStack::new(),
        }
    }

    /// Run one frame. Returns the bar's output and the activation id, if any.
    fn step(&mut self) -> (MenuBarOutput, Option<u64>) {
        self.state.begin_frame(&mut self.input);
        self.scene.begin_frame(&self.input);
        // Focus runs *after* the menu, which is the order the crate documents: the
        // menu claims the intents it handles first, so Escape with menu mode active
        // does not also blur a focused widget.
        self.focus.begin_frame(&self.input);
        let slots = self.state.push_open_layers(&mut self.layers);
        let base = self.layers.input_for_base(&self.input);
        let output = {
            let mut ctx = DrawContext::new(
                self.layers.base_mut(),
                &mut self.focus,
                &self.theme,
                &base,
                W,
                H,
            )
            .with_interactions(&mut self.scene);
            bar().draw(strip(), &mut self.state, &mut ctx)
        };
        let activated = {
            let mut env = MenuDrawEnv {
                theme: &self.theme,
                style: None,
                input: &self.input,
                focus: &mut self.focus,
                interactions: &mut self.scene,
                animations: None,
                cursor: None,
                screen_width: W,
                screen_height: H,
            };
            self.state
                .draw_open_layers(&mut self.layers, slots, MENUS, &mut env)
                .map(|item| item.id)
        };
        self.state.end_frame(&mut self.focus);
        self.scene.end_frame();
        self.focus.end_frame(None);
        self.layers.clear();
        self.input.end_frame();
        (output, activated)
    }

    /// Run frames until the open chain is painted and its regions registered.
    fn settle(&mut self) {
        self.step();
        self.step();
    }

    fn press_alt(&mut self) {
        self.input.alt_pressed = true;
        self.input.alt_down = true;
    }

    fn tap_alt(&mut self) {
        // A bare tap: press and release land in the same frame, with the key
        // already up again.
        self.input.alt_pressed = true;
        self.input.alt_released = true;
        self.input.alt_down = false;
    }

    fn move_pointer(&mut self, x: f32, y: f32) {
        self.input.mouse_x = x;
        self.input.mouse_y = y;
    }

    fn click(&mut self, x: f32, y: f32) {
        self.move_pointer(x, y);
        self.input.mouse_clicked = true;
        self.input.mouse_down = true;
    }

    /// The rect of the open column, read out of the state's promoted geometry.
    fn column_rect(&self) -> Rect {
        let geom = self.state.debug_geometry().expect("a chain is promoted");
        geom.0
    }

    /// The row height of the open column, from the same promoted geometry.
    fn row_height(&self) -> f32 {
        self.state.debug_geometry().expect("a chain is promoted").1
    }
}

// ---------------------------------------------------------------- pure geometry

#[test]
fn a_column_drops_below_its_label_left_aligned() {
    let anchor = Rect::new(20.0, 4.0, 40.0, 26.0);
    let (rect, side) = place_popup(anchor, [120.0, 100.0], viewport(), SubmenuSide::Auto);
    assert_eq!(side, SubmenuSide::Right, "Auto extends right when it fits");
    assert_eq!(rect.x, 20.0, "left edges align");
    assert_eq!(rect.y, anchor.bottom() + 2.0, "drops below the label");
    assert_eq!(rect.width, 120.0);
    assert_eq!(rect.height, 100.0);
}

#[test]
fn a_bottom_docked_bar_flips_the_column_above_its_label() {
    // A strip hugging the bottom edge has no room below it.
    let anchor = Rect::new(20.0, H - 30.0, 40.0, 28.0);
    let (rect, _) = place_popup(anchor, [120.0, 200.0], viewport(), SubmenuSide::Auto);
    assert_eq!(
        rect.bottom(),
        anchor.y - 2.0,
        "the column flips above rather than covering the strip"
    );
    assert!(rect.y >= 0.0);
}

#[test]
fn auto_flips_left_at_the_right_edge_but_an_explicit_side_slides() {
    let anchor = Rect::new(700.0, 4.0, 40.0, 26.0);
    let viewport = viewport();

    let (auto, side) = place_popup(anchor, [120.0, 50.0], viewport, SubmenuSide::Auto);
    assert_eq!(side, SubmenuSide::Left, "Auto flips to fit");
    assert_eq!(auto.right(), anchor.right(), "flipped: right edges align");

    let (right, side) = place_popup(anchor, [120.0, 50.0], viewport, SubmenuSide::Right);
    assert_eq!(side, SubmenuSide::Right, "an explicit side is honoured");
    assert_eq!(
        right.right(),
        viewport.right(),
        "…and then shifted back inside the viewport"
    );
    assert_ne!(auto.x, right.x, "flip and shift are different corrections");
}

#[test]
fn placement_respects_a_non_zero_origin_viewport() {
    let viewport = Rect::new(100.0, 50.0, 300.0, 200.0);
    let anchor = Rect::new(110.0, 60.0, 40.0, 20.0);
    let (rect, _) = place_popup(anchor, [280.0, 100.0], viewport, SubmenuSide::Auto);
    assert!(rect.x >= viewport.x);
    assert!(rect.right() <= viewport.right() + 0.001);
    assert!(rect.y >= viewport.y);
    assert!(rect.bottom() <= viewport.bottom() + 0.001);
}

#[test]
fn a_column_taller_than_the_viewport_is_pinned_to_its_top() {
    let anchor = Rect::new(20.0, 60.0, 40.0, 20.0);
    let (rect, _) = place_popup(anchor, [120.0, 5000.0], viewport(), SubmenuSide::Auto);
    assert_eq!(rect.y, 0.0, "neither side fits, so the top edge is pinned");
}

#[test]
fn a_top_docked_full_width_strip_leaves_one_blocker_region() {
    // A strip hugging the viewport's top edge has nothing above it, and the
    // zero-height band is dropped rather than registered as an empty region.
    let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
    let bar = Rect::new(0.0, 0.0, 400.0, 26.0);
    let mut out = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    let count = blocker_regions(viewport, bar, &mut out);
    assert_eq!(count, 1, "everything below the strip");
    for region in &out[..count] {
        assert!(
            region.intersection(bar).is_none(),
            "no region may cover the strip: {region:?}"
        );
    }
    assert_eq!(out[0], Rect::new(0.0, bar.bottom(), 400.0, 274.0));
}

#[test]
fn a_floating_strip_leaves_three_blocker_regions_that_tile_the_viewport() {
    // Top-docked and floating: below the strip, plus the bands either side of it
    // within its own row.
    let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
    let bar = Rect::new(100.0, 0.0, 200.0, 26.0);
    let mut out = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    let count = blocker_regions(viewport, bar, &mut out);
    assert_eq!(count, 3, "a floating strip has open space either side");
    let mut covered = 0.0;
    for region in &out[..count] {
        assert!(region.intersection(bar).is_none(), "{region:?}");
        covered += region.width * region.height;
    }
    assert_eq!(
        covered,
        viewport.width * viewport.height - bar.width * bar.height
    );
}

#[test]
fn a_mid_viewport_strip_leaves_a_region_above_and_below_it() {
    let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
    let bar = Rect::new(0.0, 100.0, 400.0, 26.0);
    let mut out = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    let count = blocker_regions(viewport, bar, &mut out);
    assert_eq!(count, 2, "above and below a full-width strip");
    assert!(out[0].bottom() <= bar.y, "{:?}", out[0]);
    assert!(out[1].y >= bar.bottom(), "{:?}", out[1]);
}

#[test]
fn a_strip_outside_the_viewport_blocks_the_whole_viewport() {
    let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);
    let bar = Rect::new(0.0, 400.0, 400.0, 26.0);
    let mut out = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    let count = blocker_regions(viewport, bar, &mut out);
    assert_eq!(count, 1);
    assert_eq!(out[0], viewport);
}

// --------------------------------------------------------------------- model

#[test]
fn item_builders_and_accessors_round_trip() {
    let item = MenuItem::new("Bold")
        .id(7)
        .checked(true)
        .enabled(false)
        .accel(Accelerator::primary(Key::Char('B')));
    assert_eq!(item.label(), "Bold");
    assert!(item.is_checked());
    assert!(!item.is_enabled(), "explicitly disabled");
    assert!(!item.is_separator());
    assert!(!item.is_submenu());
    assert_eq!(
        item.accelerator(),
        Some(Accelerator::primary(Key::Char('B')))
    );
    assert_eq!(item.shortcut_text(), None);

    let parent = MenuItem::new("Recent").with_children(RECENT);
    assert!(parent.is_submenu());
    assert_eq!(parent.children().len(), 1);

    let rule = MenuItem::separator();
    assert!(rule.is_separator());
    assert!(!rule.is_enabled(), "a separator is never activatable");
}

#[test]
fn accelerator_hints_are_written_for_both_platforms() {
    let cases = [
        (Accelerator::primary(Key::Char('s')), "Ctrl+S", "⌘S"),
        (
            Accelerator::primary_shift(Key::Char('S')),
            "Ctrl+Shift+S",
            "⌘⇧S",
        ),
        (
            Accelerator::new(
                Modifiers {
                    primary: false,
                    shift: false,
                    alt: true,
                },
                Key::F(10),
            ),
            "Alt+F10",
            "⌥F10",
        ),
        (Accelerator::primary(Key::Space), "Ctrl+Space", "⌘␣"),
        (Accelerator::primary(Key::PageDown), "Ctrl+PgDn", "⌘⇟"),
    ];
    for (accel, pc, mac) in cases {
        let mut out = String::new();
        accel.write_display(AccelPlatform::Pc, &mut out);
        assert_eq!(out, pc, "{accel:?} on PC");
        out.clear();
        accel.write_display(AccelPlatform::Mac, &mut out);
        assert_eq!(out, mac, "{accel:?} on Mac");
    }
}

#[test]
fn write_hint_appends_and_prefers_the_accelerator() {
    let mut out = String::from("> ");
    MenuItem::new("Quit")
        .accel(Accelerator::primary(Key::Char('Q')))
        .write_hint(AccelPlatform::Pc, &mut out);
    assert_eq!(
        out, "> Ctrl+Q",
        "appends, so one scratch buffer serves a column"
    );

    out.clear();
    MenuItem::new("Whatever")
        .shortcut("Ctrl+Alt+Z")
        .write_hint(AccelPlatform::Pc, &mut out);
    assert_eq!(
        out, "Ctrl+Alt+Z",
        "display-only shortcut when there is no accel"
    );
}

#[test]
#[should_panic(expected = "sets both an accelerator and a display-only shortcut")]
fn an_item_with_both_kinds_of_hint_trips_the_debug_assertion() {
    let mut out = String::new();
    MenuItem::new("Both")
        .accel(Accelerator::primary(Key::Char('B')))
        .shortcut("Ctrl+B")
        .write_hint(AccelPlatform::Pc, &mut out);
}

#[test]
fn activation_ids_prefer_the_explicit_id_and_are_stable_otherwise() {
    let file = &MENUS[0];
    let named = &file.items()[0];
    assert_eq!(super::state::activation_id(file, named), NEW_ID);

    let derived = &file.items()[1];
    let first = super::state::activation_id(file, derived);
    assert_eq!(
        first,
        super::state::activation_id(file, derived),
        "derived ids are stable"
    );
    assert_ne!(
        first,
        super::state::activation_id(&MENUS[1], &MENUS[1].items()[0]),
        "the label path namespaces them"
    );
}

#[test]
fn widget_ids_are_distinct_across_the_id_families() {
    let ids = [
        bar_label_id(BAR, 0),
        bar_label_id(BAR, 1),
        row_id(BAR, 0, None, 0, 0),
        row_id(BAR, 0, None, 1, 0),
        row_id(BAR, 1, None, 0, 0),
        row_id(BAR, 0, None, 0, 1),
        blocker_region_id(BAR, 0),
        blocker_region_id(BAR, 1),
        column_blocker_id(BAR, 0, 0),
    ];
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(a, b, "accidental id collision between {a:?} and {b:?}");
        }
    }
    assert_ne!(
        row_id(BAR, 0, None, 0, 0),
        row_id(BAR, 0, Some(1), 0, 0),
        "an explicit menu id changes its rows' ids"
    );
}

// ------------------------------------------------------------ trigger / arming

#[test]
fn an_alt_tap_arms_the_bar_and_highlights_the_first_enabled_menu() {
    let mut state = MenuBarState::new();
    let mut input = InputState {
        alt_pressed: true,
        ..InputState::default()
    };
    let mut focus = FocusState::new();
    let out = bar_frame(&mut state, &mut input, &mut focus);
    assert!(state.armed(), "a tap arms the bar");
    assert_eq!(state.highlighted_menu(), Some(0));
    assert!(out.armed);
}

#[test]
fn a_bare_alt_tap_leaves_the_bar_armed() {
    // The press and the release land in the same frame with the key already up:
    // held-only state would have missed the gesture entirely.
    let mut state = MenuBarState::new();
    let mut input = InputState {
        alt_pressed: true,
        alt_released: true,
        ..InputState::default()
    };
    let mut focus = FocusState::new();
    bar_frame(&mut state, &mut input, &mut focus);
    assert!(
        state.armed(),
        "a bare tap leaves the bar armed (decision C)"
    );
}

#[test]
fn a_held_alt_stays_armed_and_its_release_does_not_disarm() {
    // The trigger is a *tap*: the press edge arms, and a release on a later frame
    // means nothing, because the bar is a mode rather than a held modifier.
    let mut rig = Rig::new();
    rig.press_alt();
    rig.step();
    assert!(rig.state.armed());

    rig.input.alt_pressed = false;
    rig.input.alt_released = true;
    rig.input.alt_down = false;
    rig.step();
    assert!(
        rig.state.armed(),
        "releasing a held Alt leaves the bar in menu mode"
    );
}

#[test]
fn a_second_alt_tap_disarms_and_a_tap_with_a_chain_open_closes_it() {
    let mut state = MenuBarState::new();
    let mut focus = FocusState::new();
    let press = || InputState {
        alt_pressed: true,
        ..InputState::default()
    };

    bar_frame(&mut state, &mut press(), &mut focus);
    assert!(state.armed());
    bar_frame(&mut state, &mut press(), &mut focus);
    assert!(!state.armed(), "a second tap leaves menu mode");
    assert_eq!(state.highlighted_menu(), None);

    // Arm, open, then tap again: the whole chain closes and menu mode ends.
    bar_frame(&mut state, &mut press(), &mut focus);
    let mut open = InputState {
        nav: crate::NavInput {
            down: true,
            ..Default::default()
        },
        ..InputState::default()
    };
    bar_frame(&mut state, &mut open, &mut focus);
    assert_eq!(state.open_menu(), Some(0));
    bar_frame(&mut state, &mut press(), &mut focus);
    assert_eq!(state.open_menu(), None);
    assert!(!state.armed());
}

#[test]
fn arrows_walk_the_bar_wrapping_and_skipping_disabled_menus() {
    let mut state = MenuBarState::new();
    let mut focus = FocusState::new();
    state.set_trigger(MenuTrigger::AltTap);
    bar_frame(
        &mut state,
        &mut InputState {
            alt_pressed: true,
            ..InputState::default()
        },
        &mut focus,
    );
    assert_eq!(state.highlighted_menu(), Some(0));

    let right = || InputState {
        nav: crate::NavInput {
            right: true,
            ..Default::default()
        },
        ..InputState::default()
    };
    bar_frame(&mut state, &mut right(), &mut focus);
    assert_eq!(state.highlighted_menu(), Some(1));
    // Menu 2 is disabled, so the walk wraps straight back to 0.
    bar_frame(&mut state, &mut right(), &mut focus);
    assert_eq!(
        state.highlighted_menu(),
        Some(0),
        "disabled menus are skipped"
    );

    let left = || InputState {
        nav: crate::NavInput {
            left: true,
            ..Default::default()
        },
        ..InputState::default()
    };
    bar_frame(&mut state, &mut left(), &mut focus);
    assert_eq!(state.highlighted_menu(), Some(1), "wraps the other way");
}

#[test]
fn down_and_confirm_open_the_highlighted_menu_on_its_first_enabled_item() {
    for key in ["down", "confirm"] {
        let mut state = MenuBarState::new();
        let mut focus = FocusState::new();
        bar_frame(
            &mut state,
            &mut InputState {
                alt_pressed: true,
                ..InputState::default()
            },
            &mut focus,
        );
        let nav = if key == "down" {
            crate::NavInput {
                down: true,
                ..Default::default()
            }
        } else {
            crate::NavInput {
                confirm: true,
                ..Default::default()
            }
        };
        bar_frame(
            &mut state,
            &mut InputState {
                nav,
                ..InputState::default()
            },
            &mut focus,
        );
        assert_eq!(state.open_menu(), Some(0), "opened by {key}");
        assert_eq!(state.highlighted_item(), Some(0), "first enabled item");
        assert_eq!(state.open_levels(), 1);
    }
}

#[test]
fn a_disabled_menu_cannot_be_opened() {
    // Opening a disabled menu directly is refused outright...
    let mut state = MenuBarState::new();
    state.open_menu_at(MENUS, 2);
    assert_eq!(state.open_menu(), None, "menu 2 is disabled");
    assert!(!state.armed(), "and touching it does not enter menu mode");

    // ...and the bar repairs a stale highlight rather than opening it, so the
    // down/confirm that follows opens an enabled menu instead.
    let mut focus = FocusState::new();
    let mut state = MenuBarState::new();
    state.highlighted_menu = Some(2);
    bar_frame(
        &mut state,
        &mut InputState {
            alt_pressed: true,
            ..InputState::default()
        },
        &mut focus,
    );
    assert_eq!(
        state.highlighted_menu(),
        Some(0),
        "a stale highlight on a disabled menu is repaired"
    );
    bar_frame(
        &mut state,
        &mut InputState {
            nav: crate::NavInput {
                down: true,
                ..Default::default()
            },
            ..InputState::default()
        },
        &mut focus,
    );
    assert_eq!(
        state.open_menu(),
        Some(0),
        "the repaired menu is the one opened"
    );
}

#[test]
fn escape_from_armed_with_nothing_open_disarms_and_is_claimed() {
    let mut state = MenuBarState::new();
    let mut focus = FocusState::new();
    let mut input = InputState {
        alt_pressed: true,
        ..InputState::default()
    };
    bar_frame(&mut state, &mut input, &mut focus);
    assert!(state.armed(), "the tap armed the bar");

    // Escape with the bar armed but nothing open.
    input.alt_pressed = false;
    input.nav.cancel = true;
    bar_frame(&mut state, &mut input, &mut focus);
    assert!(!state.armed(), "Escape leaves menu mode");
    // The menu owned the keyboard (it was armed), so it claimed the cancel edge.
    assert!(!input.nav.cancel, "an armed bar claims the cancel edge");
}

#[test]
fn a_disarmed_idle_bar_claims_nothing_so_escape_still_reaches_focus() {
    let mut state = MenuBarState::new();
    let mut input = InputState {
        nav: crate::NavInput {
            cancel: true,
            confirm: true,
            up: true,
            next: true,
            ..Default::default()
        },
        ..InputState::default()
    };
    let mut focus = FocusState::new();
    bar_frame(&mut state, &mut input, &mut focus);
    assert!(!state.wants_keyboard());
    assert!(input.nav.cancel, "Escape is not claimed");
    assert!(input.nav.confirm);
    assert!(input.nav.up);
    assert!(input.nav.next, "Tab is never claimed");
}

#[test]
fn an_open_chain_claims_the_navigation_intents_but_never_tab() {
    let mut state = MenuBarState::new();
    state.open_menu_at(MENUS, 0);
    let mut input = InputState {
        nav: crate::NavInput {
            cancel: true,
            up: true,
            down: true,
            left: true,
            right: true,
            confirm: true,
            next: true,
            prev: true,
        },
        ..InputState::default()
    };
    state.begin_frame(&mut input);
    assert!(
        !input.nav.up
            && !input.nav.down
            && !input.nav.left
            && !input.nav.right
            && !input.nav.confirm
            && !input.nav.cancel,
        "an open chain owns the navigation intents: {nav:?}",
        nav = input.nav,
    );
    assert!(input.nav.next && input.nav.prev, "a menu is not a Tab trap");
}

// ------------------------------------------------------------ chain behaviour

#[test]
fn row_navigation_skips_separators_and_disabled_items_and_wraps() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    // FILE_ITEMS: 0 New, 1 Open, 2 separator, 3 Bold, 4 Locked(disabled),
    // 5 Open Recent, 6 Quit.
    assert_eq!(rig.state.highlighted_item(), Some(0));

    let mut seen = vec![0usize];
    for _ in 0..6 {
        rig.input.nav.down = true;
        rig.step();
        seen.push(rig.state.highlighted_item().expect("still highlighted"));
    }
    assert_eq!(
        seen,
        vec![0, 1, 3, 5, 6, 0, 1],
        "separators and disabled items are stepped over, and the walk wraps"
    );

    rig.input.nav.up = true;
    rig.step();
    assert_eq!(rig.state.highlighted_item(), Some(0), "up wraps backwards");
}

#[test]
fn a_tall_column_scrolls_to_keep_the_highlight_visible() {
    // Ten rows in a 40px-tall viewport: most of the column is below the fold.
    const MANY: &[MenuItem<'static>] = &[
        MenuItem::new("One"),
        MenuItem::new("Two"),
        MenuItem::new("Three"),
        MenuItem::new("Four"),
        MenuItem::new("Five"),
        MenuItem::new("Six"),
        MenuItem::new("Seven"),
        MenuItem::new("Eight"),
        MenuItem::new("Nine"),
        MenuItem::new("Ten"),
    ];
    const SHORT: &[Menu<'static>] = &[Menu::new("Many").with_items(MANY)];
    let theme = Theme::default();
    let mut state = MenuBarState::new();
    let mut input = InputState::default();
    let mut focus = FocusState::new();
    let mut layers = LayerStack::new();
    let mut scene = InteractionScene::new();
    let small = Rect::new(0.0, 0.0, 400.0, 40.0);

    state.open_menu_at(MENUS, 0);
    let mut frame = |state: &mut MenuBarState, input: &mut InputState| {
        state.begin_frame(input);
        scene.begin_frame(input);
        let slots = state.push_open_layers(&mut layers);
        let base = layers.input_for_base(input);
        {
            let mut ctx = DrawContext::new(layers.base_mut(), &mut focus, &theme, &base, W, 40.0)
                .with_interactions(&mut scene);
            MenuBar::new(BAR, SHORT).draw(small, state, &mut ctx);
        }
        {
            let mut env = MenuDrawEnv {
                theme: &theme,
                style: None,
                input,
                focus: &mut focus,
                interactions: &mut scene,
                animations: None,
                cursor: None,
                screen_width: W,
                screen_height: 40.0,
            };
            state.draw_open_layers(&mut layers, slots, SHORT, &mut env);
        }
        state.end_frame(&mut focus);
        scene.end_frame();
        focus.end_frame(None);
        layers.clear();
        input.end_frame();
    };

    frame(&mut state, &mut input);
    frame(&mut state, &mut input);
    let geom = state.debug_geometry().expect("painted");
    let (rect, row_h, content_h) = geom;
    assert!(
        content_h > rect.height,
        "the content overflows the viewport"
    );

    // Walk to the last row; the scroll offset has to follow it.
    for _ in 0..9 {
        input.nav.down = true;
        frame(&mut state, &mut input);
    }
    assert_eq!(state.highlighted_item(), Some(9));
    let offset = state.debug_scroll();
    assert!(offset > 0.0, "the column scrolled");
    let bottom = 9.0 * row_h - offset;
    assert!(
        bottom + row_h <= rect.height + 0.001,
        "the highlighted row is inside the column: bottom={bottom}, height={}",
        rect.height
    );
}

#[test]
fn escape_unwinds_one_level_and_keeps_the_bar_armed() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    assert_eq!(rig.state.open_levels(), 1);

    rig.input.nav.cancel = true;
    rig.step();
    assert_eq!(rig.state.open_levels(), 0, "the level closed");
    assert!(rig.state.armed(), "and the bar stays armed");
    assert_eq!(
        rig.state.highlighted_menu(),
        Some(0),
        "the bar highlight survives, so menu mode is still visible"
    );
}

#[test]
fn confirming_a_leaf_activates_it_and_closes_the_whole_chain() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    // Highlight "Open…" (index 1), then confirm.
    rig.input.nav.down = true;
    rig.step();
    assert_eq!(rig.state.highlighted_item(), Some(1));

    rig.input.nav.confirm = true;
    let (_, activated) = rig.step();
    let derived = {
        let file = &MENUS[0];
        super::state::activation_id(file, &file.items()[1])
    };
    assert_eq!(activated, Some(derived), "the derived id is reported");
    assert_eq!(rig.state.open_levels(), 0);
    assert!(!rig.state.armed(), "acting on an item ends the interaction");
}

#[test]
fn an_explicitly_identified_leaf_reports_that_id() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    rig.input.nav.confirm = true; // highlights index 0 == NEW_ID
    let (_, activated) = rig.step();
    assert_eq!(activated, Some(NEW_ID));
}

#[test]
fn a_submenu_parent_never_activates_in_this_phase() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    // Walk to "Open Recent" (index 5) and confirm it.
    for _ in 0..3 {
        rig.input.nav.down = true;
        rig.step();
    }
    assert_eq!(rig.state.highlighted_item(), Some(5));
    rig.input.nav.confirm = true;
    let (_, activated) = rig.step();
    assert_eq!(activated, None, "submenus open in a later phase");
    assert_eq!(rig.state.open_levels(), 1, "and the menu stays open");
}

#[test]
fn left_and_right_switch_menus_without_closing_the_chain() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    rig.input.nav.right = true;
    rig.step();
    assert_eq!(rig.state.open_menu(), Some(1), "moved to the next menu");
    assert_eq!(rig.state.open_levels(), 1, "the chain stays open");
    assert_eq!(
        rig.state.highlighted_item(),
        Some(0),
        "first item of the new menu"
    );

    rig.input.nav.left = true;
    rig.step();
    assert_eq!(rig.state.open_menu(), Some(0));
}

#[test]
fn a_switched_menu_paints_on_the_following_frame() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let first = rig.column_rect();
    rig.input.nav.right = true;
    rig.step();
    assert_eq!(
        rig.state.debug_geometry(),
        None,
        "the old column is not painted while the new one is unmeasured"
    );
    rig.step();
    let second = rig.column_rect();
    assert_ne!(first, second, "the new menu has its own column");
}

#[test]
fn the_press_that_opens_a_menu_does_not_select_a_row() {
    // The confirm edge that opened the menu is claimed at frame-top, and the
    // chain's own keyboard handling is gated on it having been open already.
    let mut rig = Rig::new();
    rig.tap_alt();
    rig.step();
    assert!(rig.state.armed());

    rig.input.nav.down = true;
    let (_, activated) = rig.step();
    assert_eq!(rig.state.open_levels(), 1, "opened");
    assert_eq!(activated, None, "but nothing was activated");
    assert_eq!(rig.state.highlighted_item(), Some(0));
}

// -------------------------------------------------------- pointer integration

#[test]
fn hovering_a_label_while_open_switches_menus() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let label_one = MenuBar::new(BAR, MENUS).draw(
        strip(),
        &mut rig.state,
        &mut DrawContext::new(
            &mut DrawList::new(),
            &mut rig.focus,
            &rig.theme,
            &rig.input,
            W,
            H,
        ),
    );
    let _ = label_one;
    // Registered regions from `settle` are what the next frame resolves against.
    let edit_rect = {
        let out = bar().draw(
            strip(),
            &mut rig.state,
            &mut DrawContext::new(
                &mut DrawList::new(),
                &mut rig.focus,
                &rig.theme,
                &rig.input,
                W,
                H,
            ),
        );
        out.bar_rect
    };
    // The Edit label sits right after the File label.
    let file_w = rig.state.debug_label_widths()[0];
    rig.move_pointer(edit_rect.x + file_w + 2.0, edit_rect.y + 5.0);
    rig.step();
    assert_eq!(
        rig.state.open_menu(),
        Some(1),
        "hovering another label while a chain is open switches to it"
    );
}

#[test]
fn a_press_on_the_column_but_not_a_row_is_claimed_for_focus() {
    let mut rig = Rig::new();
    rig.focus.focus(7);
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let rect = rig.column_rect();
    let row_h = rig.row_height();
    // FILE_ITEMS: 0 New, 1 Open…, 2 *separator*. The separator band belongs to the
    // column — no row is registered over it — so the press is the menu's.
    rig.click(rect.x + 4.0, rect.y + row_h * 2.5);
    rig.step();
    assert_eq!(rig.state.open_levels(), 1, "the menu stays open");
    assert!(
        rig.focus.is_focused(7),
        "and the focused widget keeps focus: the press was the menu's"
    );
}

#[test]
fn choosing_a_row_does_not_blur_the_focused_widget() {
    let mut rig = Rig::new();
    rig.focus.focus(7);
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let rect = rig.column_rect();
    let row_h = rig.row_height();
    // Row 0 is "New"; click its middle.
    rig.click(rect.x + 10.0, rect.y + row_h * 0.5);
    let (_, activated) = rig.step();
    assert_eq!(activated, Some(NEW_ID));
    assert!(
        rig.focus.is_focused(7),
        "the item acts on the focused widget, so it must not blur"
    );
}

#[test]
fn a_press_outside_closes_the_chain_and_is_swallowed() {
    let mut rig = Rig::new();
    rig.focus.focus(7);
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();

    rig.click(600.0, 400.0);
    rig.step();
    assert_eq!(rig.state.open_levels(), 0, "an outside press dismisses");
    assert!(!rig.state.armed(), "and leaves menu mode");
    assert!(
        rig.focus.is_focused(7),
        "the swallowed press must not also read as click-elsewhere"
    );
}

#[test]
fn an_idle_bar_leaves_clicks_elsewhere_to_focus() {
    let mut rig = Rig::new();
    rig.focus.focus(7);
    rig.click(600.0, 400.0);
    rig.step();
    assert!(
        !rig.focus.is_focused(7),
        "with nothing armed or open, a click is an ordinary click elsewhere"
    );
}

#[test]
fn the_viewport_blocker_excludes_the_bar_strip() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let strip = strip();

    // Outside the strip, a blocker region owns the pointer: that is what makes an
    // outside press vanish instead of reaching the widget underneath.
    rig.move_pointer(500.0, 300.0);
    rig.step();
    let blocked = (0..4).any(|index| {
        rig.scene
            .candidates()
            .contains(&blocker_region_id(BAR, index))
    });
    assert!(blocked, "the blocker covers the viewport outside the strip");

    // Inside the strip, no blocker region may cover it: the labels have to stay
    // live for hover-to-switch.
    rig.move_pointer(strip.x + 2.0, strip.y + 2.0);
    rig.step();
    assert!(
        !(0..4).any(|index| rig
            .scene
            .candidates()
            .contains(&blocker_region_id(BAR, index))),
        "no blocker region covers the strip: {:?}",
        rig.scene.candidates()
    );
    assert_eq!(
        rig.scene.candidates().first().copied(),
        Some(bar_label_id(BAR, 0)),
        "the strip's own label wins there"
    );
}

#[test]
fn a_row_outranks_the_column_blocker_under_the_pointer() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let rect = rig.column_rect();

    // The third frame resolves against the regions the second registered.
    rig.move_pointer(rect.x + 4.0, rect.y + 4.0);
    rig.step();

    let row = row_id(BAR, 0, None, 0, 0);
    let blocker = column_blocker_id(BAR, 0, 0);
    let candidates = rig.scene.candidates();
    let row_rank = candidates
        .iter()
        .position(|id| *id == row)
        .unwrap_or_else(|| panic!("row 0 is under the pointer: {candidates:?}"));
    let blocker_rank = candidates
        .iter()
        .position(|id| *id == blocker)
        .unwrap_or_else(|| panic!("the column blocker is under the pointer: {candidates:?}"));
    assert!(
        row_rank < blocker_rank,
        "rows must rank above the column blocker: {candidates:?}"
    );
    assert_eq!(
        rig.scene.winner(),
        Some(row),
        "and the row takes the pointer"
    );
}

#[test]
fn the_chain_keeps_its_geometry_buffers_across_frames() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    let first = rig.state.scratch_capacities();
    assert!(first.1 > 0, "rows were measured");
    for _ in 0..3 {
        rig.step();
        assert_eq!(
            rig.state.scratch_capacities(),
            first,
            "the geometry path must reuse its buffers rather than regrow them"
        );
    }
}

#[test]
fn painting_needs_a_promoted_geometry_so_a_fresh_menu_waits_a_frame() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    // Frame 1: the bar measures the chain; nothing is painted yet.
    let (_, activated) = rig.step();
    assert_eq!(activated, None);
    assert!(rig.state.debug_geometry().is_none());
    // Frame 2: the chain is promoted and painted.
    rig.step();
    assert!(rig.state.debug_geometry().is_some());
}

#[test]
fn closing_and_reopening_clears_the_chain_geometry() {
    let mut rig = Rig::new();
    rig.state.open_menu_at(MENUS, 0);
    rig.settle();
    rig.state.close();
    rig.step();
    assert!(rig.state.debug_geometry().is_none());
    assert_eq!(rig.state.open_levels(), 0);
}

#[test]
fn scroll_and_highlight_reset_when_a_menu_opens() {
    let mut state = MenuBarState::new();
    state.open_menu_at(MENUS, 0);
    // Scroll the column and move the highlight away from the first row.
    state.scroll = 12.0;
    state.highlighted_item = Some(3);

    // Re-opening the same menu (the bar re-runs `open_menu_at` whenever it is
    // asked to open one) starts it at the top again...
    state.open_menu_at(MENUS, 0);
    assert_eq!(
        state.highlighted_item(),
        Some(0),
        "the highlight starts fresh"
    );
    assert_eq!(
        state.debug_scroll(),
        0.0,
        "and the column is back at its top"
    );

    // ...but a frame that merely *draws* the open chain leaves both alone.
    state.highlighted_item = Some(3);
    state.scroll = 12.0;
    let mut focus = FocusState::new();
    let mut list = DrawList::new();
    let mut scene = InteractionScene::new();
    let input = InputState::default();
    let theme = Theme::default();
    let mut ctx =
        DrawContext::new(&mut list, &mut focus, &theme, &input, W, H).with_interactions(&mut scene);
    bar().draw(strip(), &mut state, &mut ctx);
    assert_eq!(
        state.highlighted_item(),
        Some(3),
        "re-drawing an already-open menu must not reset its highlight"
    );
    assert_eq!(state.debug_scroll(), 12.0, "nor its scroll offset");
}

#[test]
fn the_bar_measures_as_one_row_tall() {
    use crate::{MeasureConstraints, MeasureContext, StyleResolver};
    let theme = Theme::default();
    let styles = StyleResolver::new(&theme);
    let mut text = crate::TextMeasurer::default();
    let constraints = MeasureConstraints::UNBOUNDED;
    let mut cx = MeasureContext::new(
        &mut text,
        styles,
        crate::FontSpec::default(),
        constraints,
        1.0,
        crate::WrapMode::None,
    );
    let measured = bar().measure(&mut cx);
    assert_eq!(measured.preferred[1], theme.menu_row_height);
    assert_eq!(measured.max[1], Some(theme.menu_row_height));
    assert!(
        measured.preferred[0] > 0.0,
        "the strip is as wide as its labels"
    );
}
