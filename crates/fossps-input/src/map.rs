//! The action map: named actions and the inputs bound to them.

use std::fmt;
use std::ops::Range;

use crate::binding::{ActionKind, Binding};
use crate::names::{UnknownName, unknown};
use crate::shaping::Deadzone;

/// A handle to an action in an [`ActionMap`].
///
/// Look an action up by name once, keep the id, and use it every tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(usize);

impl ActionId {
    /// Position of the action in its map.
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// One named action and where its bindings live in the map.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Action {
    pub(crate) name: Box<str>,
    pub(crate) kind: ActionKind,
    pub(crate) bindings: Range<usize>,
    pub(crate) stick_curve: f32,
}

/// Named actions and the keys, buttons and sticks that drive them.
///
/// ```
/// use fossps_input::ActionMap;
///
/// let mut map = ActionMap::new();
/// map.button("jump", ["Space", "GamepadSouth"])?;
/// map.axis2d("move", ["WASD", "LeftStick"])?;
/// map.axis2d("look", ["MouseMotion", "RightStick"])?;
/// # Ok::<(), fossps_input::MapError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActionMap {
    actions: Vec<Action>,
    bindings: Vec<Binding>,
    deadzone: Deadzone,
}

impl ActionMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The standard first-person shooter controls, for keyboard and mouse and for gamepad.
    ///
    /// | Action | Keyboard and mouse | Gamepad |
    /// | --- | --- | --- |
    /// | `move` | WASD | left stick |
    /// | `look` | mouse | right stick |
    /// | `jump` | Space | south button |
    /// | `crouch` | Left Control | east button |
    /// | `sprint` | Left Shift | left stick press |
    /// | `fire` | left mouse button | right trigger |
    /// | `aim` | right mouse button | left trigger |
    /// | `reload` | R | west button |
    /// | `interact` | E | north button |
    /// | `melee` | V | right stick press |
    /// | `grenade` | G | right bumper |
    /// | `next_weapon` | wheel down | d-pad right |
    /// | `previous_weapon` | wheel up | d-pad left |
    /// | `scoreboard` | Tab | select |
    /// | `pause` | Escape | start |
    #[must_use]
    pub fn default_fps() -> Self {
        const BUTTONS: &[(&str, [&str; 2])] = &[
            ("jump", ["Space", "GamepadSouth"]),
            ("crouch", ["LeftControl", "GamepadEast"]),
            ("sprint", ["LeftShift", "LeftStickPress"]),
            ("fire", ["MouseLeft", "RightTrigger"]),
            ("aim", ["MouseRight", "LeftTrigger"]),
            ("reload", ["R", "GamepadWest"]),
            ("interact", ["E", "GamepadNorth"]),
            ("melee", ["V", "RightStickPress"]),
            ("grenade", ["G", "RightBumper"]),
            ("next_weapon", ["MouseWheelDown", "DpadRight"]),
            ("previous_weapon", ["MouseWheelUp", "DpadLeft"]),
            ("scoreboard", ["Tab", "GamepadSelect"]),
            ("pause", ["Escape", "GamepadStart"]),
        ];
        let mut map = Self::new();
        // The names and bindings above are fixed and covered by a test, so none
        // of these calls can fail; ignoring the results keeps this infallible.
        let _ = map.axis2d("move", ["WASD", "LeftStick"]);
        if let Ok(look) = map.axis2d("look", ["MouseMotion", "RightStick"]) {
            map.set_stick_curve(look, 2.0);
        }
        for (name, bindings) in BUTTONS {
            let _ = map.button(name, bindings);
        }
        map
    }

    /// Adds a button action, such as jump or fire.
    ///
    /// Bind keys, mouse buttons, wheel notches and gamepad buttons by name:
    /// `map.button("jump", ["Space", "GamepadSouth"])`.
    ///
    /// # Errors
    ///
    /// Fails if the name is taken or empty, or if a binding is misspelled or is
    /// not a button.
    pub fn button<I>(&mut self, name: &str, bindings: I) -> Result<ActionId, MapError>
    where
        I: IntoIterator<Item: AsRef<str>>,
    {
        self.add_named(name, ActionKind::Button, bindings)
    }

    /// Adds a 1D axis action, a number from -1 to 1, such as lean.
    ///
    /// Bind pairs of buttons or one axis of a stick. In a pair the button that
    /// pushes toward -1 comes first, so `"Q/E"` leans left with Q:
    /// `map.axis1d("lean", ["Q/E", "LeftStickX"])`.
    ///
    /// # Errors
    ///
    /// Fails if the name is taken or empty, or if a binding is misspelled or is
    /// not a 1D axis.
    pub fn axis1d<I>(&mut self, name: &str, bindings: I) -> Result<ActionId, MapError>
    where
        I: IntoIterator<Item: AsRef<str>>,
    {
        self.add_named(name, ActionKind::Axis1D, bindings)
    }

    /// Adds a 2D axis action, a direction, such as move or look.
    ///
    /// Bind four-button groups, sticks or the mouse. A custom group lists its
    /// buttons in up/down/left/right order, such as `"I/K/J/L"`:
    /// `map.axis2d("move", ["WASD", "LeftStick"])`.
    ///
    /// # Errors
    ///
    /// Fails if the name is taken or empty, or if a binding is misspelled or is
    /// not a 2D axis.
    pub fn axis2d<I>(&mut self, name: &str, bindings: I) -> Result<ActionId, MapError>
    where
        I: IntoIterator<Item: AsRef<str>>,
    {
        self.add_named(name, ActionKind::Axis2D, bindings)
    }

    /// Parses binding names, then adds the action.
    fn add_named<I>(
        &mut self,
        name: &str,
        kind: ActionKind,
        bindings: I,
    ) -> Result<ActionId, MapError>
    where
        I: IntoIterator<Item: AsRef<str>>,
    {
        let start = self.bindings.len();
        for text in bindings {
            match text.as_ref().parse() {
                Ok(binding) => self.bindings.push(binding),
                Err(source) => {
                    self.bindings.truncate(start);
                    return Err(MapError::UnknownBinding {
                        action: name.to_owned(),
                        source,
                    });
                }
            }
        }
        self.finish_action(name, kind, start)
    }

    /// Adds an action from already-built [`Binding`] values.
    ///
    /// This is for engine code that builds bindings itself. Games normally use
    /// [`button`](Self::button), [`axis1d`](Self::axis1d) and [`axis2d`](Self::axis2d).
    ///
    /// # Errors
    ///
    /// Fails if the name is taken or empty, or if a binding does not match `kind`.
    pub fn add<I>(
        &mut self,
        name: &str,
        kind: ActionKind,
        bindings: I,
    ) -> Result<ActionId, MapError>
    where
        I: IntoIterator<Item = Binding>,
    {
        let start = self.bindings.len();
        self.bindings.extend(bindings);
        self.finish_action(name, kind, start)
    }

    /// Validates the action whose bindings start at `start`, and records it.
    ///
    /// On failure the map is left exactly as it was before the bindings were pushed.
    fn finish_action(
        &mut self,
        name: &str,
        kind: ActionKind,
        start: usize,
    ) -> Result<ActionId, MapError> {
        let problem = if name.trim().is_empty() {
            Some(MapError::EmptyName)
        } else if self.actions.iter().any(|action| &*action.name == name) {
            Some(MapError::DuplicateAction {
                action: name.to_owned(),
            })
        } else {
            self.bindings[start..]
                .iter()
                .find(|binding| binding.kind() != kind)
                .map(|binding| MapError::WrongKind {
                    action: name.to_owned(),
                    kind,
                    binding: *binding,
                })
        };
        if let Some(problem) = problem {
            self.bindings.truncate(start);
            return Err(problem);
        }
        self.actions.push(Action {
            name: name.into(),
            kind,
            bindings: start..self.bindings.len(),
            stick_curve: 1.0,
        });
        Ok(ActionId(self.actions.len() - 1))
    }

    /// Finds an action by name.
    ///
    /// # Errors
    ///
    /// Fails if no action has that name; the error suggests the closest one.
    pub fn action(&self, name: &str) -> Result<ActionId, UnknownName> {
        self.actions
            .iter()
            .position(|action| &*action.name == name)
            .map(ActionId)
            .ok_or_else(|| unknown("action", name, self.actions.iter().map(|a| &*a.name)))
    }

    /// Every action in the map, in the order they were added.
    pub fn actions(&self) -> impl ExactSizeIterator<Item = ActionId> + use<> {
        (0..self.actions.len()).map(ActionId)
    }

    /// How many actions the map has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Whether the map has no actions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// The action's name, or `""` for an id from a different map.
    #[must_use]
    pub fn name(&self, action: ActionId) -> &str {
        self.actions.get(action.0).map_or("", |a| &a.name)
    }

    /// The action's kind, or `None` for an id from a different map.
    #[must_use]
    pub fn kind(&self, action: ActionId) -> Option<ActionKind> {
        self.actions.get(action.0).map(|a| a.kind)
    }

    /// The inputs bound to the action.
    #[must_use]
    pub fn bindings(&self, action: ActionId) -> &[Binding] {
        self.actions
            .get(action.0)
            .and_then(|a| self.bindings.get(a.bindings.clone()))
            .unwrap_or_default()
    }

    /// Sets how sticks bound to this action respond to tilt.
    ///
    /// 1 is linear, the default. Higher values give finer control near the
    /// centre: 2 is a good choice for aiming. Values that are not numbers and
    /// ids from a different map are ignored.
    pub fn set_stick_curve(&mut self, action: ActionId, curve: f32) {
        if let Some(action) = self.actions.get_mut(action.0)
            && curve.is_finite()
        {
            action.stick_curve = curve.max(0.01);
        }
    }

    /// The dead zone applied to both thumbsticks.
    #[must_use]
    pub fn deadzone(&self) -> Deadzone {
        self.deadzone
    }

    /// Changes the dead zone applied to both thumbsticks.
    pub fn set_deadzone(&mut self, deadzone: Deadzone) {
        if deadzone.inner.is_finite() && deadzone.outer.is_finite() {
            self.deadzone = deadzone;
        }
    }

    /// Lists every action a player on one kind of device could not use.
    ///
    /// FOSSPS games are playable on a gamepad and on keyboard and mouse, so
    /// every action needs a binding on each. An empty list means the map is
    /// complete. Each [`ParityGap`] prints as a sentence saying what to add.
    #[must_use]
    pub fn parity_gaps(&self) -> Vec<ParityGap> {
        let mut gaps = Vec::new();
        for action in &self.actions {
            let bindings = &self.bindings[action.bindings.clone()];
            for (missing, has) in [
                (Device::Gamepad, bindings.iter().any(|b| b.is_gamepad())),
                (
                    Device::KeyboardAndMouse,
                    bindings.iter().any(|b| !b.is_gamepad()),
                ),
            ] {
                if !has {
                    gaps.push(ParityGap {
                        action: action.name.to_string(),
                        kind: action.kind,
                        missing,
                    });
                }
            }
        }
        gaps
    }

    /// The actions and flat binding list, for the tick loop.
    pub(crate) fn parts(&self) -> (&[Action], &[Binding]) {
        (&self.actions, &self.bindings)
    }
}

/// A kind of input device a player might be using.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {
    /// Keyboard and mouse.
    KeyboardAndMouse,
    /// A gamepad.
    Gamepad,
}

/// An action that players on one kind of device cannot use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParityGap {
    /// The action's name.
    pub action: String,
    /// The action's kind.
    pub kind: ActionKind,
    /// The device with no binding for it.
    pub missing: Device,
}

impl fmt::Display for ParityGap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (device, players, example) = match (self.missing, self.kind) {
            (Device::Gamepad, ActionKind::Button) => ("gamepad", "gamepad", "GamepadSouth"),
            (Device::Gamepad, ActionKind::Axis1D) => ("gamepad", "gamepad", "LeftStickX"),
            (Device::Gamepad, ActionKind::Axis2D) => ("gamepad", "gamepad", "LeftStick"),
            (Device::KeyboardAndMouse, ActionKind::Button) => {
                ("keyboard or mouse", "keyboard and mouse", "Space")
            }
            (Device::KeyboardAndMouse, ActionKind::Axis1D) => {
                ("keyboard or mouse", "keyboard and mouse", "Q/E")
            }
            (Device::KeyboardAndMouse, ActionKind::Axis2D) => {
                ("keyboard or mouse", "keyboard and mouse", "WASD")
            }
        };
        write!(
            f,
            "Action \"{}\" has no {device} binding, so {players} players cannot use it. \
             Add one, for example \"{example}\".",
            self.action
        )
    }
}

/// Why an action could not be added to an [`ActionMap`].
#[derive(Debug, Clone, PartialEq)]
pub enum MapError {
    /// The action name was empty.
    EmptyName,
    /// An action with this name already exists.
    DuplicateAction {
        /// The repeated name.
        action: String,
    },
    /// A binding name was not recognised.
    UnknownBinding {
        /// The action being added.
        action: String,
        /// Which name failed, with a suggestion when one is close.
        source: UnknownName,
    },
    /// A binding produces a different kind of value than the action expects.
    WrongKind {
        /// The action being added.
        action: String,
        /// The action's kind.
        kind: ActionKind,
        /// The binding that does not fit.
        binding: Binding,
    },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => f.write_str("An action needs a name, for example \"jump\"."),
            Self::DuplicateAction { action } => write!(
                f,
                "Action \"{action}\" is defined twice. \
                 Put all of its bindings in one list, or give the second action a different name."
            ),
            Self::UnknownBinding { action, source } => {
                write!(f, "Action \"{action}\": {source}")
            }
            Self::WrongKind {
                action,
                kind,
                binding,
            } => {
                let fix = match binding.kind() {
                    ActionKind::Button => "button",
                    ActionKind::Axis1D => "axis1d",
                    ActionKind::Axis2D => "axis2d",
                };
                write!(
                    f,
                    "Action \"{action}\" is {}, but \"{binding}\" is {}. \
                     Pick a different binding, or declare the action with `{fix}`.",
                    kind.describe(),
                    binding.kind().describe(),
                )
            }
        }
    }
}

impl std::error::Error for MapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownBinding { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{Key, Stick};

    #[test]
    fn default_map_is_complete_on_both_devices() {
        let map = ActionMap::default_fps();
        assert_eq!(map.len(), 15);
        assert_eq!(map.parity_gaps(), Vec::new());
        for name in ["move", "look", "jump", "fire", "pause", "previous_weapon"] {
            assert!(map.action(name).is_ok(), "missing {name}");
        }
        let look = map.action("look").unwrap();
        assert_eq!(map.kind(look), Some(ActionKind::Axis2D));
    }

    #[test]
    fn actions_keep_their_bindings() {
        let mut map = ActionMap::new();
        let jump = map.button("jump", ["Space", "GamepadSouth"]).unwrap();
        let movement = map.axis2d("move", ["WASD", "LeftStick"]).unwrap();
        assert_eq!(map.name(jump), "jump");
        assert_eq!(map.bindings(jump).len(), 2);
        assert_eq!(
            map.bindings(movement),
            [
                Binding::directions(Key::W, Key::S, Key::A, Key::D),
                Binding::Stick(Stick::LeftStick)
            ]
        );
        assert_eq!(map.actions().collect::<Vec<_>>(), [jump, movement]);
    }

    #[test]
    fn unknown_action_suggests_the_closest() {
        let map = ActionMap::default_fps();
        let error = map.action("jmup").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"jmup\" is not a known action. Did you mean \"jump\"?"
        );
    }

    #[test]
    fn misspelled_binding_is_explained_and_leaves_the_map_unchanged() {
        let mut map = ActionMap::new();
        let error = map.button("jump", ["Space", "GamepadSuoth"]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Action \"jump\": \"GamepadSuoth\" is not a known binding. \
             Did you mean \"GamepadSouth\"?"
        );
        assert_eq!(map, ActionMap::new());
    }

    #[test]
    fn wrong_kind_is_explained_and_leaves_the_map_unchanged() {
        let mut map = ActionMap::new();
        let error = map.button("jump", ["LeftStick"]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Action \"jump\" is a button, but \"LeftStick\" is a 2D axis (a direction). \
             Pick a different binding, or declare the action with `axis2d`."
        );
        assert_eq!(map, ActionMap::new());
    }

    #[test]
    fn duplicate_and_empty_names_are_rejected() {
        let mut map = ActionMap::new();
        map.button("jump", ["Space"]).unwrap();
        assert!(matches!(
            map.button("jump", ["Enter"]),
            Err(MapError::DuplicateAction { .. })
        ));
        assert_eq!(map.button(" ", ["Enter"]), Err(MapError::EmptyName));
        assert_eq!(map.len(), 1);
        assert_eq!(map.bindings(map.action("jump").unwrap()).len(), 1);
    }

    #[test]
    fn parity_gaps_say_what_to_add() {
        let mut map = ActionMap::new();
        map.button("jump", ["Space"]).unwrap();
        map.axis2d("look", ["RightStick"]).unwrap();
        let gaps: Vec<String> = map.parity_gaps().iter().map(ToString::to_string).collect();
        assert_eq!(
            gaps,
            [
                "Action \"jump\" has no gamepad binding, so gamepad players cannot use it. \
                 Add one, for example \"GamepadSouth\".",
                "Action \"look\" has no keyboard or mouse binding, so keyboard and mouse players \
                 cannot use it. Add one, for example \"WASD\".",
            ]
        );
    }

    #[test]
    fn foreign_ids_are_harmless() {
        let map = ActionMap::new();
        let foreign = ActionId(7);
        assert_eq!(map.name(foreign), "");
        assert_eq!(map.kind(foreign), None);
        assert!(map.bindings(foreign).is_empty());
    }
}
