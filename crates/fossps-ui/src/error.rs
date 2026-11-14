//! What can go wrong while building or changing a HUD or menu.

use std::fmt;

/// A mistake in how a HUD or menu was built or changed.
///
/// Every message says what to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum UiError {
    /// A widget id that this [`Ui`](crate::Ui) never handed out.
    UnknownWidget,
    /// A screen id that this [`Ui`](crate::Ui) never handed out.
    UnknownScreen,
    /// A setter was used on a widget of another kind.
    WrongKind {
        /// The kind of widget the setter changes, for example `"slider"`.
        expected: &'static str,
        /// The kind of widget the id belongs to.
        found: &'static str,
    },
    /// A slider whose range or step cannot work.
    BadSlider {
        /// The slider's label.
        label: String,
    },
    /// A choice or a tab bar with nothing to pick from.
    NoOptions {
        /// The widget's label, or `"tabs"` for a tab bar.
        label: String,
    },
    /// A grid with no columns.
    NoColumns,
    /// A neighbor that is not a widget on the screen being built.
    NeighborNotOnScreen,
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWidget => f.write_str(
                "That widget does not belong to this UI. Keep the value that `button`, `label` \
                 and the other builders return, and use it with the UI that made it.",
            ),
            Self::UnknownScreen => f.write_str(
                "That screen does not belong to this UI. Keep the value `add_screen` returns, \
                 and use it with the UI that made it.",
            ),
            Self::WrongKind { expected, found } => write!(
                f,
                "This widget is a {found}, not a {expected}. Use the setter that matches the \
                 builder that made it."
            ),
            Self::BadSlider { label } => write!(
                f,
                "The slider \"{label}\" needs a lowest value below its highest value, and a \
                 step above zero. For example: lowest 0, highest 100, step 5."
            ),
            Self::NoOptions { label } => {
                write!(
                    f,
                    "\"{label}\" has nothing to pick from. Give it at least one name."
                )
            }
            Self::NoColumns => f.write_str("A grid needs at least one column."),
            Self::NeighborNotOnScreen => f.write_str(
                "A neighbor must be a button, toggle, slider or choice on the same screen, \
                 made before `neighbor` is called.",
            ),
        }
    }
}

impl std::error::Error for UiError {}
