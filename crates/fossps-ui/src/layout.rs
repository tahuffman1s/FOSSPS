//! Working out where everything goes.
//!
//! All of this is in "reference pixels": pixels on a screen 1080 high.
//! The draw list is scaled to real pixels at the very end.

use crate::node::{Kind, NONE, Node, Rect, children};
use crate::theme::{SMALLEST_TEXT, TextMeasure, Theme};

/// Text is never drawn smaller than [`SMALLEST_TEXT`].
pub(crate) fn text_size(size: f32) -> f32 {
    size.max(SMALLEST_TEXT)
}

/// Height of the box a line of text sits in.
fn line_height(size: f32) -> f32 {
    size * 1.25
}

/// Width of one tab in a tab bar.
pub(crate) fn tab_width(name: &str, theme: &Theme, measure: &dyn TextMeasure) -> f32 {
    measure.width(name, text_size(theme.text_size)) + 2.0 * theme.padding
}

/// The part of a slider the handle travels along.
pub(crate) fn slider_track(rect: Rect, theme: &Theme) -> Rect {
    let (w, h) = (200.0, 8.0);
    Rect {
        x: rect.right() - theme.padding - 90.0 - w,
        y: rect.y + (rect.h - h) / 2.0,
        w,
        h,
    }
}

/// Fills in the size every widget of a screen wants. Children come after
/// their parents in the list, so going backwards sizes them first.
pub(crate) fn measure_screen(
    nodes: &mut [Node],
    first: usize,
    end: usize,
    theme: &Theme,
    measure: &dyn TextMeasure,
) {
    let body = text_size(theme.text_size);
    for index in (first..end).rev() {
        let node = &nodes[index];
        #[allow(
            clippy::cast_possible_truncation,
            reason = "node counts fit in u32 by construction"
        )]
        let kids = || children(nodes, index as u32).map(|child| nodes[child as usize].size);
        let size = match &node.kind {
            Kind::Label { title } => {
                let size = if *title {
                    text_size(theme.title_size)
                } else {
                    body
                };
                [measure.width(&node.text, size), line_height(size)]
            }
            Kind::Button => [
                (measure.width(&node.text, body) + 2.0 * theme.padding).max(theme.button_width),
                theme.row_height,
            ],
            Kind::Toggle { .. } | Kind::Slider { .. } | Kind::Choice { .. } => {
                [theme.setting_width, theme.row_height]
            }
            Kind::Column { gap } => {
                let mut size = [0.0_f32, -gap];
                for kid in kids() {
                    size = [size[0].max(kid[0]), size[1] + kid[1] + gap];
                }
                [size[0], size[1].max(0.0)]
            }
            Kind::Row { gap } => {
                let mut size = [-gap, 0.0_f32];
                for kid in kids() {
                    size = [size[0] + kid[0] + gap, size[1].max(kid[1])];
                }
                [size[0].max(0.0), size[1]]
            }
            Kind::Grid { columns, gap } => {
                let mut cell = [0.0_f32; 2];
                let mut count = 0_u32;
                for kid in kids() {
                    cell = [cell[0].max(kid[0]), cell[1].max(kid[1])];
                    count += 1;
                }
                let rows = count.div_ceil(*columns);
                [span(cell[0], *columns, *gap), span(cell[1], rows, *gap)]
            }
            Kind::Tabs { names, .. } => {
                let mut bar = -theme.gap;
                for name in names {
                    bar += tab_width(name, theme, measure) + theme.gap;
                }
                let mut page = [0.0_f32; 2];
                for kid in kids() {
                    page = [page[0].max(kid[0]), page[1].max(kid[1])];
                }
                [bar.max(page[0]), theme.row_height + theme.gap + page[1]]
            }
            Kind::HudText { .. } | Kind::HudBar { .. } | Kind::HudRect { .. } => continue,
        };
        nodes[index].size = size;
    }
}

/// The length of `count` cells in a line with gaps between them.
fn span(cell: f32, count: u32, gap: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "a grid has far fewer than 2^24 cells"
    )]
    let count = count as f32;
    cell * count + gap * (count - 1.0)
}

/// What a container does with its children.
enum Plan {
    Column(f32),
    Row(f32),
    Grid(u32, f32),
    Tabs(usize),
    Leaf,
}

/// Places every widget of a screen, given where its first node goes.
/// Parents come before their children, so going forwards places each
/// container before what is inside it.
pub(crate) fn place_screen(nodes: &mut [Node], first: usize, end: usize, theme: &Theme) {
    for index in first..end {
        let (rect, visible) = (nodes[index].rect, nodes[index].visible);
        let plan = match &nodes[index].kind {
            Kind::Column { gap } => Plan::Column(*gap),
            Kind::Row { gap } => Plan::Row(*gap),
            Kind::Grid { columns, gap } => Plan::Grid(*columns, *gap),
            Kind::Tabs { open, .. } => Plan::Tabs(*open),
            _ => Plan::Leaf,
        };
        let mut cell = [0.0_f32; 2];
        if let Plan::Grid(columns, gap) = plan {
            #[allow(clippy::cast_possible_truncation, reason = "node counts fit in u32")]
            for child in children(nodes, index as u32) {
                cell[1] = cell[1].max(nodes[child as usize].size[1]);
            }
            #[allow(clippy::cast_precision_loss, reason = "a grid has few columns")]
            let across = columns as f32;
            cell[0] = (rect.w - gap * (across - 1.0)) / across;
        }

        let mut child = nodes[index].first_child;
        let mut place = 0_u32;
        let mut cursor = [rect.x, rect.y];
        while child != NONE {
            let kid = &mut nodes[child as usize];
            kid.visible = visible;
            match plan {
                Plan::Column(gap) => {
                    kid.rect = Rect {
                        x: rect.x,
                        y: cursor[1],
                        w: rect.w,
                        h: kid.size[1],
                    };
                    cursor[1] += kid.size[1] + gap;
                }
                Plan::Row(gap) => {
                    kid.rect = Rect {
                        x: cursor[0],
                        y: rect.y,
                        w: kid.size[0],
                        h: rect.h,
                    };
                    cursor[0] += kid.size[0] + gap;
                }
                Plan::Grid(columns, gap) => {
                    kid.rect = Rect {
                        x: cursor[0],
                        y: cursor[1],
                        w: cell[0],
                        h: cell[1],
                    };
                    if (place + 1).is_multiple_of(columns) {
                        cursor = [rect.x, cursor[1] + cell[1] + gap];
                    } else {
                        cursor[0] += cell[0] + gap;
                    }
                }
                Plan::Tabs(open) => {
                    let top = theme.row_height + theme.gap;
                    kid.rect = Rect {
                        x: rect.x,
                        y: rect.y + top,
                        w: rect.w,
                        h: rect.h - top,
                    };
                    kid.visible = visible && place as usize == open;
                }
                Plan::Leaf => {}
            }
            place += 1;
            child = kid.next_sibling;
        }
    }
}

/// Pins one HUD piece to its anchor inside the safe area.
pub(crate) fn place_hud(node: &mut Node, safe: Rect, measure: &dyn TextMeasure) {
    let (anchor, offset) = match &node.kind {
        Kind::HudText {
            anchor,
            offset,
            size,
            ..
        } => {
            let size = text_size(*size);
            node.size = [measure.width(&node.text, size), size];
            (*anchor, *offset)
        }
        Kind::HudBar { anchor, offset, .. } | Kind::HudRect { anchor, offset, .. } => {
            (*anchor, *offset)
        }
        _ => return,
    };
    let [across, down] = anchor.fractions();
    node.rect = Rect {
        x: safe.x + across * (safe.w - node.size[0]) + offset[0],
        y: safe.y + down * (safe.h - node.size[1]) + offset[1],
        w: node.size[0],
        h: node.size[1],
    };
}
