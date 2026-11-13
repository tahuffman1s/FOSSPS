//! The per-tick snapshot of every action.

use crate::map::ActionId;

/// What one action is doing during one tick.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ActionState {
    /// The action is active: its button is held, or its axis is off centre.
    pub pressed: bool,
    /// The action became active this tick.
    pub just_pressed: bool,
    /// The action stopped being active this tick.
    pub just_released: bool,
    /// The action's value. Buttons and 1D axes use only the first number.
    pub axis: [f32; 2],
    /// Mouse movement since the previous tick, in raw counts: right and up are positive.
    pub delta: [f32; 2],
}

/// Every action's state for one simulation tick.
///
/// This is plain data. The simulation reads it, and it is also what gets
/// recorded for replays and sent to the server.
#[derive(Debug, Default, PartialEq)]
pub struct InputFrame {
    /// Which tick this is. The first tick is 1; 0 means no tick has run yet.
    pub tick: u64,
    pub(crate) actions: Vec<ActionState>,
}

impl Clone for InputFrame {
    fn clone(&self) -> Self {
        Self {
            tick: self.tick,
            actions: self.actions.clone(),
        }
    }

    /// Copies `source` into this frame, reusing its memory: a frame that is
    /// refreshed every tick this way does not allocate.
    fn clone_from(&mut self, source: &Self) {
        self.tick = source.tick;
        self.actions.clone_from(&source.actions);
    }
}

impl InputFrame {
    /// A frame with `actions` idle actions.
    pub(crate) fn idle(actions: usize) -> Self {
        Self {
            tick: 0,
            actions: vec![ActionState::default(); actions],
        }
    }

    /// The full state of an action. An id from a different map reads as idle.
    #[must_use]
    pub fn get(&self, action: ActionId) -> ActionState {
        self.actions
            .get(action.index())
            .copied()
            .unwrap_or_default()
    }

    /// Whether the action is active: a held button, or an axis off centre.
    #[must_use]
    pub fn pressed(&self, action: ActionId) -> bool {
        self.get(action).pressed
    }

    /// Whether the action became active this tick. True for exactly one tick per press.
    #[must_use]
    pub fn just_pressed(&self, action: ActionId) -> bool {
        self.get(action).just_pressed
    }

    /// Whether the action stopped being active this tick.
    #[must_use]
    pub fn just_released(&self, action: ActionId) -> bool {
        self.get(action).just_released
    }

    /// The action as one number.
    ///
    /// For a button this is 0 to 1 (how far a trigger is pulled; keys are 0 or 1).
    /// For a 1D axis it is -1 to 1.
    #[must_use]
    pub fn value(&self, action: ActionId) -> f32 {
        self.get(action).axis[0]
    }

    /// The action as a direction: X and Y from -1 to 1, never longer than 1.
    ///
    /// Right and up are positive. Mouse movement is not included; read it with
    /// [`delta`](Self::delta).
    #[must_use]
    pub fn axis2d(&self, action: ActionId) -> [f32; 2] {
        self.get(action).axis
    }

    /// Mouse movement bound to the action since the previous tick, in raw counts.
    ///
    /// Right and up are positive. Multiply by a mouse sensitivity, but not by
    /// the tick length: counts already measure how far the mouse moved. A stick
    /// bound to the same action reads through [`axis2d`](Self::axis2d) as a
    /// rate, which does need multiplying by the tick length.
    #[must_use]
    pub fn delta(&self, action: ActionId) -> [f32; 2] {
        self.get(action).delta
    }
}
