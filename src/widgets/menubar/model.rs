//! The borrowed menu description: [`Menu`], [`MenuItem`], accelerators, and the
//! [`WidgetId`] derivation that namespaces both into one
//! [`InteractionScene`](crate::InteractionScene).
//!
//! Every constructor here is `const fn`, so a whole menu tree can live in a
//! `static`/`const` slice with no per-frame building and no allocation:
//!
//! ```ignore
//! const FILE_ITEMS: &[MenuItem<'static>] = &[
//!     MenuItem::new("New").accel(Accelerator::primary(Key::Char('N'))).id(FILE_NEW),
//!     MenuItem::separator(),
//!     MenuItem::new("Quit").accel(Accelerator::primary(Key::Char('Q'))).id(FILE_QUIT),
//! ];
//! const MENUS: &[Menu<'static>] = &[Menu::new("File").items(FILE_ITEMS)];
//! ```
//!
//! The description is **not** state: `checked`/`enabled` are the values to render
//! for the borrow the caller hands in this frame. A host whose toggles change
//! keeps its own model and borrows a per-frame view of it (rebuilt only when its
//! model changes, never per frame). [`super::MenuBarState`] never stores or
//! mutates item status, which is what keeps the crate's caller-owned-state
//! contract intact.

use crate::WidgetId;

/// Namespace for one menubar's widget ids within the surface's shared
/// [`InteractionScene`](crate::InteractionScene). Required, and must be unique
/// per bar — two bars sharing an id would have their rows dispatch as one.
pub type MenuBarId = u64;

/// Stable identity for one item within a bar. This is what a caller matches on
/// when an item activates ([`super::ActivatedItem::id`]).
pub type MenuItemId = u64;

/// Which side a popup column extends to. For submenu flyouts it is literally the
/// side of the parent row; for a top-level column — which drops *below* its bar
/// label — it is which way the column extends from the label, i.e. whether it is
/// left-aligned or right-aligned with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SubmenuSide {
    /// Extend right, flipping left when that would overflow the viewport.
    #[default]
    Auto,
    /// Always extend right, then shift back inside the viewport.
    Right,
    /// Always extend left, then shift back inside the viewport.
    Left,
}

/// Word or glyph set used to render an accelerator hint. Display only: it never
/// affects which keys match.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AccelPlatform {
    /// `Ctrl+Shift+S` style hints.
    #[default]
    Pc,
    /// `⌘⇧S` style hints.
    Mac,
}

/// What arms the bar.
///
/// Arming is a *mode*, not a held key: once armed, navigation is ordinary
/// arrow/Enter keys and the trigger need not stay held. A bare tap therefore
/// leaves the bar armed until something disarms it (a second tap, Escape with
/// nothing open, or a click that closes the chain).
///
/// The trigger lives in [`super::MenuBarState`] rather than the widget because
/// arming is evaluated at frame-top, before the bar draws — so the bar can render
/// armed on the very frame the tap lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MenuTrigger {
    /// A tap of Alt (Option) arms the bar. The default.
    ///
    /// Alt keeps press *and* release edges on
    /// [`InputState`](crate::InputState) because a bare tap landing entirely
    /// between two rendered frames is a gesture in its own right: held-only state
    /// would never see it.
    #[default]
    AltTap,
    // `KeyTap(Key)` — a tap of a plain key, `F10` being the other classic menu
    // key — arrives with the generic per-frame key edges (design doc phase 4).
    // The crate models no generic key list yet, so the variant is not declared:
    // an enum arm that can never fire is worse than a missing one.
}

/// A key on its own, independent of modifiers. Used by [`Accelerator`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// An ASCII character. Case is folded by the host before mapping, so `'A'`
    /// and `'a'` are the same key.
    Char(char),
    /// A function key, `F(1)`..=`F(24)`.
    F(u8),
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Enter/Return.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Insert.
    Insert,
    /// Space.
    Space,
}

/// Modifier state for an [`Accelerator`].
///
/// Only three modifiers are modelled: "the platform's primary command modifier"
/// rather than Ctrl and Cmd separately, matching
/// [`InputState::ctrl_pressed`](crate::InputState::ctrl_pressed)'s existing
/// documentation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Modifiers {
    /// Ctrl on PC, ⌘ (Command) on Mac.
    pub primary: bool,
    /// Shift.
    pub shift: bool,
    /// Alt (Option on Mac).
    pub alt: bool,
}

/// One keyboard shortcut: the modifiers and key that trigger it.
///
/// A single value drives both the rendered hint and the host's matching, so the
/// two cannot diverge — which is the whole reason it is a type rather than two
/// strings. The lib renders hints and exposes [`matches`](Self::matches); the
/// *host* owns dispatch, scanning its own tree when a key press is present.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Accelerator {
    /// Required modifier state — matched exactly, so an extra held modifier does
    /// not match.
    pub mods: Modifiers,
    /// The key, without modifiers.
    pub key: Key,
}

impl Accelerator {
    /// An accelerator requiring exactly `mods` and `key`.
    pub const fn new(mods: Modifiers, key: Key) -> Self {
        Self { mods, key }
    }

    /// Primary modifier + `key`: Ctrl on PC, ⌘ on Mac.
    pub const fn primary(key: Key) -> Self {
        Self {
            mods: Modifiers {
                primary: true,
                shift: false,
                alt: false,
            },
            key,
        }
    }

    /// Primary + Shift + `key`.
    pub const fn primary_shift(key: Key) -> Self {
        Self {
            mods: Modifiers {
                primary: true,
                shift: true,
                alt: false,
            },
            key,
        }
    }

    /// Write the canonical hint for this accelerator into `out` (which is
    /// **appended to**, so a caller can reuse one scratch `String` across rows).
    ///
    /// Modifiers are written primary → alt → shift on both platforms, e.g.
    /// `Ctrl+Shift+S` (`Modifiers::primary_shift`) and `⌘⇧S` on Mac.
    pub fn write_display(&self, platform: AccelPlatform, out: &mut String) {
        match platform {
            AccelPlatform::Pc => {
                if self.mods.primary {
                    out.push_str("Ctrl+");
                }
                if self.mods.alt {
                    out.push_str("Alt+");
                }
                if self.mods.shift {
                    out.push_str("Shift+");
                }
            }
            AccelPlatform::Mac => {
                if self.mods.primary {
                    out.push('⌘');
                }
                if self.mods.alt {
                    out.push('⌥');
                }
                if self.mods.shift {
                    out.push('⇧');
                }
            }
        }
        match (platform, self.key) {
            (_, Key::Char(ch)) => out.push(ch.to_ascii_uppercase()),
            (_, Key::F(n)) => {
                out.push('F');
                push_u8(out, n);
            }
            (AccelPlatform::Pc, key) => out.push_str(pc_key_name(key)),
            (AccelPlatform::Mac, key) => out.push_str(mac_key_name(key)),
        }
    }
}

/// Decimal formatting without `format!` (the hint path is a hot loop and a
/// scratch buffer, so no per-row allocation beyond the hint's own content).
fn push_u8(out: &mut String, n: u8) {
    let mut buf = [0u8; 3];
    let mut len = 0;
    let mut value = n;
    loop {
        buf[len] = b'0' + (value % 10);
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for byte in buf[..len].iter().rev() {
        out.push(*byte as char);
    }
}

fn pc_key_name(key: Key) -> &'static str {
    match key {
        Key::Char(_) | Key::F(_) => "",
        Key::Left => "Left",
        Key::Right => "Right",
        Key::Up => "Up",
        Key::Down => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PgUp",
        Key::PageDown => "PgDn",
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::Delete => "Del",
        Key::Insert => "Ins",
        Key::Space => "Space",
    }
}

fn mac_key_name(key: Key) -> &'static str {
    match key {
        Key::Char(_) | Key::F(_) => "",
        Key::Left => "←",
        Key::Right => "→",
        Key::Up => "↑",
        Key::Down => "↓",
        Key::Home => "↖",
        Key::End => "↘",
        Key::PageUp => "⇞",
        Key::PageDown => "⇟",
        Key::Enter => "⏎",
        Key::Escape => "⎋",
        Key::Tab => "⇥",
        Key::Backspace => "⌫",
        Key::Delete => "⌦",
        Key::Insert => "⌤",
        Key::Space => "␣",
    }
}

/// A single entry in a menu: an action, a submenu parent, or a separator.
///
/// Leaf items of a column never activate when disabled or separated; a parent
/// opens its children (a later phase) rather than activating.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    id: Option<MenuItemId>,
    label: &'a str,
    children: &'a [MenuItem<'a>],
    separator: bool,
    enabled: bool,
    checked: bool,
    accel: Option<Accelerator>,
    accel_text: Option<&'a str>,
}

impl<'a> MenuItem<'a> {
    /// An enabled, unchecked leaf with `label`.
    pub const fn new(label: &'a str) -> Self {
        Self {
            id: None,
            label,
            children: &[],
            separator: false,
            enabled: true,
            checked: false,
            accel: None,
            accel_text: None,
        }
    }

    /// A horizontal rule. Never highlighted, never activatable.
    pub const fn separator() -> Self {
        Self {
            id: None,
            label: "",
            children: &[],
            separator: true,
            enabled: false,
            checked: false,
            accel: None,
            accel_text: None,
        }
    }

    /// Pin this item's activation id. Without one, the id is derived from the
    /// item's label path — stable, but not something a caller can name in
    /// advance. Explicit ids are what callers match on.
    pub const fn id(mut self, id: MenuItemId) -> Self {
        self.id = Some(id);
        self
    }

    /// Make this a submenu parent whose child column is `items`.
    pub const fn with_children(mut self, items: &'a [MenuItem<'a>]) -> Self {
        self.children = items;
        self
    }

    /// Enable or disable the item. Disabled items are dimmed and never
    /// activatable, highlighted or hovered.
    pub const fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Render a check mark in the item's check gutter.
    pub const fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    /// Attach an [`Accelerator`]: it both renders the hint and is the value the
    /// host matches on.
    pub const fn accel(mut self, accel: Accelerator) -> Self {
        self.accel = Some(accel);
        self
    }

    /// Display-only hint text, for a shortcut the *host* handles and this crate
    /// cannot describe (a game action, a chord with modifiers the lib doesn't
    /// model). Prefer [`accel`](Self::accel); setting both is a bug and trips a
    /// debug assertion.
    pub const fn shortcut(mut self, text: &'a str) -> Self {
        self.accel_text = Some(text);
        self
    }

    /// The item's label.
    pub fn label(&self) -> &'a str {
        self.label
    }

    /// Whether this is a separator rather than an item.
    pub fn is_separator(&self) -> bool {
        self.separator
    }

    /// Whether the item can be activated or highlighted.
    pub fn is_enabled(&self) -> bool {
        self.enabled && !self.separator
    }

    /// Whether the item is checked (and so draws its check mark).
    pub fn is_checked(&self) -> bool {
        self.checked
    }

    /// Whether this item opens a child column rather than activating.
    pub fn is_submenu(&self) -> bool {
        !self.children.is_empty()
    }

    /// The child column this item opens, empty for a leaf.
    pub fn children(&self) -> &'a [MenuItem<'a>] {
        self.children
    }

    /// The accelerator attached to this item, if any.
    pub fn accelerator(&self) -> Option<Accelerator> {
        self.accel
    }

    /// The display-only shortcut hint attached to this item, if any.
    pub fn shortcut_text(&self) -> Option<&'a str> {
        self.accel_text
    }

    /// The hint to render for this item, appended to `out`: the accelerator's
    /// canonical display when one is set, else the display-only shortcut text.
    ///
    /// Writing into a caller-owned buffer is what lets one scratch `String` serve
    /// every row of a column.
    pub(crate) fn write_hint(&self, platform: AccelPlatform, out: &mut String) {
        if let Some(accel) = self.accel {
            debug_assert!(
                self.accel_text.is_none(),
                "menu item {:?} sets both an accelerator and a display-only shortcut; \
                 the accelerator wins (the rendered hint must be the value that matches)",
                self.label
            );
            accel.write_display(platform, out);
        } else if let Some(text) = self.accel_text {
            out.push_str(text);
        }
    }

    /// The item's activation id: the explicit one, else one derived from its
    /// label path within the menu, so an item the caller never named still has a
    /// stable identity to report.
    pub(crate) fn activation_id(&self, path: &[&str]) -> MenuItemId {
        match self.id {
            Some(id) => id,
            None => {
                let mut acc = FNV_BASIS ^ Seed::Activation as u64;
                for step in path {
                    acc = hash_key(acc, step);
                }
                acc
            }
        }
    }
}

/// One top-level menu: its bar label plus the item column it drops down.
#[derive(Clone, Copy, Debug)]
pub struct Menu<'a> {
    id: Option<MenuItemId>,
    label: &'a str,
    items: &'a [MenuItem<'a>],
    enabled: bool,
}

impl<'a> Menu<'a> {
    /// An enabled menu with `label` and no items.
    pub const fn new(label: &'a str) -> Self {
        Self {
            id: None,
            label,
            items: &[],
            enabled: true,
        }
    }

    /// The items this menu drops down.
    pub const fn with_items(mut self, items: &'a [MenuItem<'a>]) -> Self {
        self.items = items;
        self
    }

    /// Pin this menu's identity contribution to its rows' [`WidgetId`]s.
    ///
    /// Without one, a menu contributes its index in the bar — so *reordering*
    /// the bar renames every row id of the menus that moved. Give menus stable
    /// ids if their order can change while a chain is open.
    pub const fn id(mut self, id: MenuItemId) -> Self {
        self.id = Some(id);
        self
    }

    /// Enable or disable the whole menu: a disabled menu dims, never opens, and
    /// is skipped by keyboard traversal of the bar.
    pub const fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// The bar label.
    pub fn label(&self) -> &'a str {
        self.label
    }

    /// The menu's items.
    pub fn items(&self) -> &'a [MenuItem<'a>] {
        self.items
    }

    /// Whether the menu can be opened.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The menu's explicit id, if the caller pinned one.
    pub fn id_value(&self) -> Option<MenuItemId> {
        self.id
    }

    /// The first item that can be highlighted, if any.
    pub(crate) fn first_enabled_item(&self) -> Option<usize> {
        self.items.iter().position(|item| item.is_enabled())
    }
}

/// One item's activation, as resolved by
/// [`super::MenuBarState::draw_open_layers`]. This is the single place an
/// activation is reported, so there is no ambiguity about which result to read.
#[derive(Clone, Copy, Debug)]
pub struct ActivatedItem<'a> {
    /// The item's activation id: its explicit `.id()`, else one derived from its
    /// label path.
    pub id: MenuItemId,
    /// The item itself, for callers that want the whole description.
    pub item: &'a MenuItem<'a>,
}

/// Namespace seeds. Each family of derived ids hashes a distinct seed first, so
/// a bar label can never collide with a row or a blocker region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Seed {
    /// A bar label's hit region.
    BarLabel = 1,
    /// A menu row's hit region.
    Row = 2,
    /// The viewport blocker, registered as scene regions.
    BlockerRegion = 3,
    /// One column's own full-rect blocker region.
    ColumnBlocker = 4,
    /// An item's derived activation id.
    Activation = 5,
}

/// FNV-1a over `key`'s bytes, continuing from `acc`.
///
/// Cheap, allocation-free, stable across frames and processes — all the id
/// derivation needs. These ids are never persisted, so hash quality only has to
/// avoid *accidental* collisions within one bar.
fn hash_key(acc: u64, key: &str) -> u64 {
    let mut acc = acc;
    for byte in key.as_bytes() {
        acc ^= *byte as u64;
        acc = acc.wrapping_mul(FNV_PRIME);
    }
    acc
}

/// Continue an FNV-1a fold with one integer.
fn hash_u64(acc: u64, value: u64) -> u64 {
    let mut acc = acc;
    for byte in value.to_le_bytes() {
        acc ^= byte as u64;
        acc = acc.wrapping_mul(FNV_PRIME);
    }
    acc
}

/// FNV-1a offset basis, and prime.
const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The [`WidgetId`] of one bar label.
pub(crate) fn bar_label_id(bar: MenuBarId, menu_index: usize) -> WidgetId {
    WidgetId(hash_u64(
        hash_u64(FNV_BASIS ^ Seed::BarLabel as u64, bar),
        menu_index as u64,
    ))
}

/// The [`WidgetId`] of one menu row.
///
/// Derived from the bar id, the menu's index *and* its explicit id, the item
/// index and the level: unique by construction, stable across frames while the
/// tree shape holds, and independent of label text.
pub(crate) fn row_id(
    bar: MenuBarId,
    menu_index: usize,
    menu_id: Option<MenuItemId>,
    item_index: usize,
    level: usize,
) -> WidgetId {
    let mut acc = FNV_BASIS ^ Seed::Row as u64;
    acc = hash_u64(acc, bar);
    acc = hash_u64(acc, menu_index as u64);
    acc = hash_u64(acc, menu_id.unwrap_or(0));
    acc = hash_u64(acc, item_index as u64);
    acc = hash_u64(acc, level as u64);
    WidgetId(acc)
}

/// The [`WidgetId`] of the `index`-th viewport-blocker region.
pub(crate) fn blocker_region_id(bar: MenuBarId, index: usize) -> WidgetId {
    WidgetId(hash_u64(
        hash_u64(FNV_BASIS ^ Seed::BlockerRegion as u64, bar),
        index as u64,
    ))
}

/// The [`WidgetId`] of one column's own blocker region.
pub(crate) fn column_blocker_id(bar: MenuBarId, menu_index: usize, level: usize) -> WidgetId {
    let mut acc = FNV_BASIS ^ Seed::ColumnBlocker as u64;
    acc = hash_u64(acc, bar);
    acc = hash_u64(acc, menu_index as u64);
    acc = hash_u64(acc, level as u64);
    WidgetId(acc)
}
