//! Turning placed widgets into boxes and text.

use crate::draw::Painter;
use crate::layout::{slider_track, tab_width, text_size};
use crate::node::{Kind, Node, Rect};
use crate::theme::{TextMeasure, Theme};

/// How a widget is shown this frame.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    /// The player is on it, or the pointer is over it.
    pub selected: bool,
    /// It gets the gamepad and keyboard outline.
    pub outlined: bool,
}

pub(crate) fn hud(node: &Node, theme: &Theme, painter: &mut Painter<'_>) {
    let rect = node.rect;
    match &node.kind {
        Kind::HudText { size, color, .. } => {
            painter.text([rect.x, rect.y], text_size(*size), *color, &node.text);
        }
        Kind::HudBar {
            fraction, color, ..
        } => {
            painter.rect(rect, theme.track, 0.0);
            painter.rect(
                Rect {
                    w: rect.w * fraction,
                    ..rect
                },
                *color,
                0.0,
            );
        }
        Kind::HudRect { color, .. } => painter.rect(rect, *color, 0.0),
        _ => {}
    }
}

pub(crate) fn widget(
    node: &Node,
    look: Look,
    theme: &Theme,
    measure: &dyn TextMeasure,
    painter: &mut Painter<'_>,
) {
    let rect = node.rect;
    let size = text_size(theme.text_size);
    let color = if look.selected {
        theme.text_selected
    } else {
        theme.text
    };
    let text_y = rect.y + (rect.h - size) / 2.0;
    let right = rect.right() - theme.padding;
    let background = |painter: &mut Painter<'_>| {
        if look.outlined {
            painter.rect(
                rect.grown(theme.outline),
                theme.accent,
                theme.corner + theme.outline,
            );
        }
        let fill = if look.selected {
            theme.widget_selected
        } else {
            theme.widget
        };
        painter.rect(rect, fill, theme.corner);
    };
    let label = |painter: &mut Painter<'_>| {
        painter.text([rect.x + theme.padding, text_y], size, color, &node.text);
    };

    match &node.kind {
        Kind::Label { title } => {
            let size = if *title {
                text_size(theme.title_size)
            } else {
                size
            };
            painter.text(
                [rect.x, rect.y + (rect.h - size) / 2.0],
                size,
                theme.text,
                &node.text,
            );
        }
        Kind::Button => {
            background(painter);
            let width = measure.width(&node.text, size);
            painter.text(
                [rect.x + (rect.w - width) / 2.0, text_y],
                size,
                color,
                &node.text,
            );
        }
        Kind::Toggle { on } => {
            background(painter);
            label(painter);
            let word = if *on { "On" } else { "Off" };
            painter.text(
                [right - measure.width(word, size), text_y],
                size,
                color,
                word,
            );
        }
        Kind::Slider {
            min,
            max,
            step,
            value,
        } => {
            background(painter);
            label(painter);
            let track = slider_track(rect, theme);
            let filled = track.w * ((value - min) / (max - min)).clamp(0.0, 1.0);
            painter.rect(track, theme.track, track.h / 2.0);
            painter.rect(Rect { w: filled, ..track }, theme.accent, track.h / 2.0);
            let handle = Rect {
                x: track.x + filled - 6.0,
                y: rect.y + rect.h / 2.0 - 12.0,
                w: 12.0,
                h: 24.0,
            };
            painter.rect(handle, color, 4.0);
            let decimals = if *step >= 1.0 {
                0
            } else if *step >= 0.1 {
                1
            } else {
                2
            };
            painter.text_with(
                [track.right() + 16.0, text_y],
                size,
                color,
                format_args!("{value:.decimals$}"),
            );
        }
        Kind::Choice { options, selected } => {
            background(painter);
            label(painter);
            let left = right - 280.0;
            let arrow = measure.width(">", size);
            painter.text([left, text_y], size, color, "<");
            painter.text([right - arrow, text_y], size, color, ">");
            let option = options.get(*selected).map_or("", String::as_str);
            let middle = (left + right) / 2.0;
            painter.text(
                [middle - measure.width(option, size) / 2.0, text_y],
                size,
                color,
                option,
            );
        }
        Kind::Tabs { names, open } => {
            let mut x = rect.x;
            for (index, name) in names.iter().enumerate() {
                let tab = Rect {
                    x,
                    y: rect.y,
                    w: tab_width(name, theme, measure),
                    h: theme.row_height,
                };
                let is_open = index == *open;
                let fill = if is_open {
                    theme.widget_selected
                } else {
                    theme.widget
                };
                painter.rect(tab, fill, theme.corner);
                if is_open {
                    let line = Rect {
                        y: tab.bottom() - theme.outline,
                        h: theme.outline,
                        ..tab
                    };
                    painter.rect(line, theme.accent, 0.0);
                }
                let color = if is_open {
                    theme.text_selected
                } else {
                    theme.text
                };
                let y = tab.y + (tab.h - size) / 2.0;
                painter.text([x + theme.padding, y], size, color, name);
                x += tab.w + theme.gap;
            }
        }
        _ => {}
    }
}
