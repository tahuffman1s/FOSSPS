//! The widgets themselves: one flat list, linked into trees.

use crate::theme::Color;

/// "No widget", for the links between nodes.
pub(crate) const NONE: u32 = u32::MAX;

/// Names one widget or HUD piece, so it can be changed later and
/// recognised in a [`UiEvent`](crate::UiEvent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(pub(crate) u32);

/// Names one menu screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenId(pub(crate) u32);

/// Where on the screen a HUD piece is pinned.
///
/// A piece pinned to `BottomRight` keeps its bottom-right corner in the
/// bottom-right corner of the screen at every resolution and shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs, reason = "each variant is the place of the same name")]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// How far across and down the screen the anchor is, each 0, a half or 1.
    pub(crate) const fn fractions(self) -> [f32; 2] {
        match self {
            Self::TopLeft => [0.0, 0.0],
            Self::Top => [0.5, 0.0],
            Self::TopRight => [1.0, 0.0],
            Self::Left => [0.0, 0.5],
            Self::Center => [0.5, 0.5],
            Self::Right => [1.0, 0.5],
            Self::BottomLeft => [0.0, 1.0],
            Self::Bottom => [0.5, 1.0],
            Self::BottomRight => [1.0, 1.0],
        }
    }
}

/// A box on the screen, measured from the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub(crate) fn right(self) -> f32 {
        self.x + self.w
    }

    pub(crate) fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub(crate) fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.x
            && point[0] < self.right()
            && point[1] >= self.y
            && point[1] < self.bottom()
    }

    /// The same box, `by` bigger on every side.
    pub(crate) fn grown(self, by: f32) -> Self {
        Self {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }
}

/// What a node is, with the values only that kind has.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Kind {
    Column {
        gap: f32,
    },
    Row {
        gap: f32,
    },
    Grid {
        columns: u32,
        gap: f32,
    },
    Label {
        title: bool,
    },
    Button,
    Toggle {
        on: bool,
    },
    Slider {
        min: f32,
        max: f32,
        step: f32,
        value: f32,
    },
    Choice {
        options: Vec<String>,
        selected: usize,
    },
    /// Each child is one tab's page, in the order of `names`.
    Tabs {
        names: Vec<String>,
        open: usize,
    },
    HudText {
        anchor: Anchor,
        offset: [f32; 2],
        size: f32,
        color: Color,
    },
    HudBar {
        anchor: Anchor,
        offset: [f32; 2],
        fraction: f32,
        color: Color,
    },
    HudRect {
        anchor: Anchor,
        offset: [f32; 2],
        color: Color,
    },
}

impl Kind {
    /// The name used in error messages.
    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Column { .. } => "column",
            Self::Row { .. } => "row",
            Self::Grid { .. } => "grid",
            Self::Label { .. } => "label",
            Self::Button => "button",
            Self::Toggle { .. } => "toggle",
            Self::Slider { .. } => "slider",
            Self::Choice { .. } => "choice",
            Self::Tabs { .. } => "tab bar",
            Self::HudText { .. } => "HUD text",
            Self::HudBar { .. } => "HUD bar",
            Self::HudRect { .. } => "HUD rectangle",
        }
    }

    /// Whether a gamepad or keyboard can land on this kind of widget.
    pub(crate) const fn takes_focus(&self) -> bool {
        matches!(
            self,
            Self::Button | Self::Toggle { .. } | Self::Slider { .. } | Self::Choice { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub kind: Kind,
    /// The words on the widget. Empty for containers and shapes.
    pub text: String,
    pub first_child: u32,
    pub last_child: u32,
    pub next_sibling: u32,
    /// The size the widget wants, before its container places it. For a
    /// HUD bar or rectangle, the size it was given.
    pub size: [f32; 2],
    /// Where the widget ended up.
    pub rect: Rect,
    /// False for widgets on a tab that is not open.
    pub visible: bool,
    /// Where each direction goes when the author said so, indexed by
    /// `Direction::index`.
    pub neighbors: [u32; 4],
}

impl Node {
    pub(crate) fn new(kind: Kind, text: &str) -> Self {
        Self {
            kind,
            text: text.to_owned(),
            first_child: NONE,
            last_child: NONE,
            next_sibling: NONE,
            size: [0.0; 2],
            rect: Rect::default(),
            visible: true,
            neighbors: [NONE; 4],
        }
    }

    /// Whether a gamepad or keyboard can land on this widget right now.
    pub(crate) fn focusable(&self) -> bool {
        self.visible && self.kind.takes_focus()
    }
}

/// One menu screen: a run of nodes whose first is the screen's column.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Screen {
    pub root: u32,
    /// One past the screen's last node.
    pub end: u32,
    /// The widget the player is on, remembered while the screen is closed.
    pub focus: u32,
    /// The widget the player starts on, if the author chose one.
    pub start: u32,
    pub wrap: bool,
    pub stays_open: bool,
}

/// The children of `parent`, first to last.
pub(crate) fn children(nodes: &[Node], parent: u32) -> impl Iterator<Item = u32> + '_ {
    let mut next = nodes[parent as usize].first_child;
    std::iter::from_fn(move || {
        if next == NONE {
            return None;
        }
        let child = next;
        next = nodes[child as usize].next_sibling;
        Some(child)
    })
}
