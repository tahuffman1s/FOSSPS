//! The editor's panels: what is in the level on the left, what is known
//! about the chosen thing on the right, and how to fly along the bottom.
//!
//! They are made with `egui` and are for the mouse. What the level file
//! holds is shown and cannot be changed; what is placed in the map file can
//! be added, changed and removed.

use std::fmt::Write as _;

use fossps_assets::{Bounds, LightKind, Scene, Transform};

use super::place::Snap;
use crate::map::{self, MAP_FILE, Map};

/// How wide the two side panels start out, in `egui` points.
const LIST_WIDTH: f32 = 260.0;
const INSPECTOR_WIDTH: f32 = 300.0;

/// How to fly, said along the bottom of the window.
const KEYS: &str = "Look: hold the right mouse button    Fly: W A S D    Up, down: E, Q    \
                    Faster: hold Shift    Speed: mouse wheel";
const EDIT_KEYS: &str = "Move: drag a mark (Alt: up and down, Ctrl: off the grid)    \
                         Turn: drag the tip of its line, or Z and C    Place: P    \
                         Carry, put down: F    Put back: Esc    Snap: G";
const EDIT_GAMEPAD: &str = "A places where you look, X carries what you look at and puts it \
                            down, B puts it back, LB and RB turn it, Y changes the snap";
const GAMEPAD: &str = "Gamepad: the sticks fly and look, the triggers go down and up, \
                       pressing the left stick is faster, d-pad up and down change the speed";

/// The sorts of thing a level holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A shape placed in the level.
    Object,
    /// A named place with no shape, such as a spawn point.
    Marker,
    /// A light.
    Light,
}

impl Kind {
    /// The word shown beside a name in the list.
    fn word(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Marker => "marker",
            Self::Light => "light",
        }
    }
}

/// One thing in the level, as the panels show it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entity {
    pub(crate) name: String,
    pub(crate) kind: Kind,
    /// The middle of the space it takes up, in metres.
    pub(crate) middle: [f32; 3],
    /// How far it reaches from its middle, in metres.
    pub(crate) reach: f32,
    /// What the inspector says about it: what, then the value.
    facts: Vec<(&'static str, String)>,
    /// Its colours, as sRGB bytes.
    colors: Vec<[u8; 3]>,
    /// The custom properties given to it in the 3D tool, as JSON text.
    properties: Option<String>,
}

/// `x, y, z` in metres, to the centimetre.
fn metres([x, y, z]: [f32; 3]) -> String {
    format!("{x:.2}, {y:.2}, {z:.2} m")
}

/// Where `transform` puts `point`.
fn placed(transform: &Transform, point: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|row| {
        transform[0][row] * point[0]
            + transform[1][row] * point[1]
            + transform[2][row] * point[2]
            + transform[3][row]
    })
}

/// How far something placed by `transform` is turned to the left and tilted
/// up, in degrees, going by where its front (-Z) points.
fn facing(transform: &Transform) -> String {
    let [x, y, z] = [-transform[2][0], -transform[2][1], -transform[2][2]];
    let flat = x.hypot(z);
    // The engine's yaw: 0 faces -Z and it grows turning left.
    let turned = if flat > 1e-6 {
        (-x).atan2(-z).to_degrees()
    } else {
        0.0
    };
    // Whole degrees, and never "-0".
    let (turned, tilted) = (
        turned.round() + 0.0,
        y.atan2(flat).to_degrees().round() + 0.0,
    );
    let mut words = format!("{turned:.0}° to the left");
    if tilted != 0.0 {
        // Writing to a `String` cannot fail.
        let _ = write!(words, ", tilted {tilted:.0}° up");
    }
    words
}

/// A colour in linear light as the sRGB bytes a screen shows.
fn shown([red, green, blue]: [f32; 3]) -> [u8; 3] {
    let color = egui::Color32::from(egui::Rgba::from_rgb(
        red.clamp(0.0, 1.0),
        green.clamp(0.0, 1.0),
        blue.clamp(0.0, 1.0),
    ));
    [color.r(), color.g(), color.b()]
}

/// The name given in the 3D tool, or the sort of thing and its number.
fn name_of(name: Option<&String>, kind: Kind, number: usize) -> String {
    match name {
        Some(name) if !name.trim().is_empty() => name.clone(),
        _ => format!("Unnamed {} {}", kind.word(), number + 1),
    }
}

/// Everything in `scene`, in the order it is listed: objects, markers, lights.
pub(crate) fn entities(scene: &Scene) -> Vec<Entity> {
    let mut all = Vec::with_capacity(scene.instances.len() + scene.markers.len());
    for (number, instance) in scene.instances.iter().enumerate() {
        let mesh = scene.meshes.get(instance.mesh as usize);
        let position = placed(&instance.transform, [0.0; 3]);
        // The box around it once placed: every corner of every part's box.
        let mut around: Option<Bounds> = None;
        let mut triangles = 0;
        let mut colors = Vec::new();
        for part in mesh.into_iter().flat_map(|mesh| &mesh.parts) {
            triangles += part.indices.len() / 3;
            if let Some(material) = scene.materials.get(part.material as usize) {
                let [red, green, blue, _] = material.base_color;
                let color = shown([red, green, blue]);
                if !colors.contains(&color) && colors.len() < 8 {
                    colors.push(color);
                }
            }
            for corner in 0..8 {
                let pick = |axis: usize| {
                    if corner >> axis & 1 == 0 {
                        part.bounds.min[axis]
                    } else {
                        part.bounds.max[axis]
                    }
                };
                let point = placed(&instance.transform, [pick(0), pick(1), pick(2)]);
                let bounds = around.get_or_insert(Bounds {
                    min: point,
                    max: point,
                });
                for (axis, place) in point.into_iter().enumerate() {
                    bounds.min[axis] = bounds.min[axis].min(place);
                    bounds.max[axis] = bounds.max[axis].max(place);
                }
            }
        }
        let around = around.unwrap_or(Bounds {
            min: position,
            max: position,
        });
        let size = [0, 1, 2].map(|axis| around.max[axis] - around.min[axis]);
        let mut facts = vec![
            ("Position", metres(position)),
            (
                "Size",
                format!(
                    "{:.2} wide, {:.2} high, {:.2} deep (m)",
                    size[0], size[1], size[2]
                ),
            ),
            ("Turned", facing(&instance.transform)),
        ];
        let shape = mesh.and_then(|mesh| mesh.name.as_deref()).unwrap_or("");
        facts.push((
            "Shape",
            if shape.is_empty() {
                format!("{triangles} triangles")
            } else {
                format!("{shape}, {triangles} triangles")
            },
        ));
        if instance.mirrored {
            facts.push(("Mirrored", "yes".to_owned()));
        }
        all.push(Entity {
            // An object with no name of its own goes by its shape's.
            name: name_of(
                instance
                    .name
                    .as_ref()
                    .or(mesh.and_then(|mesh| mesh.name.as_ref())),
                Kind::Object,
                number,
            ),
            kind: Kind::Object,
            middle: [0, 1, 2].map(|axis| (around.min[axis] + around.max[axis]) / 2.0),
            reach: size.iter().map(|side| side * side).sum::<f32>().sqrt() / 2.0,
            facts,
            colors,
            properties: instance.extras.clone(),
        });
    }
    for (number, marker) in scene.markers.iter().enumerate() {
        let position = placed(&marker.transform, [0.0; 3]);
        all.push(Entity {
            name: name_of(marker.name.as_ref(), Kind::Marker, number),
            kind: Kind::Marker,
            middle: position,
            reach: 1.0,
            facts: vec![
                ("Position", metres(position)),
                ("Turned", facing(&marker.transform)),
            ],
            colors: Vec::new(),
            properties: marker.extras.clone(),
        });
    }
    for (number, light) in scene.lights.iter().enumerate() {
        let (sort, unit) = match light.kind {
            LightKind::Sun => ("Sun", "lux"),
            LightKind::Point => ("Point light", "candela"),
            LightKind::Spot { .. } => ("Spot light", "candela"),
        };
        let mut facts = vec![("Sort", sort.to_owned())];
        if !matches!(light.kind, LightKind::Sun) {
            facts.push(("Position", metres(light.position)));
        }
        if !matches!(light.kind, LightKind::Point) {
            let [x, y, z] = light.direction;
            facts.push(("Shines along", format!("{x:.2}, {y:.2}, {z:.2}")));
        }
        if let LightKind::Spot { outer_angle, .. } = light.kind {
            facts.push((
                "Cone",
                format!("{:.0}° across", (outer_angle * 2.0).to_degrees()),
            ));
        }
        facts.push(("Brightness", format!("{:.0} {unit}", light.intensity)));
        facts.push((
            "Reach",
            light
                .range
                .map_or_else(|| "no limit".to_owned(), |range| format!("{range:.1} m")),
        ));
        all.push(Entity {
            name: name_of(light.name.as_ref(), Kind::Light, number),
            kind: Kind::Light,
            middle: light.position,
            reach: 1.0,
            facts,
            colors: vec![shown(light.color)],
            properties: None,
        });
    }
    all
}

/// An `egui` set up for the editor: dark, and with the engine's own font,
/// so no second font is carried.
pub(crate) fn context() -> egui::Context {
    let context = egui::Context::default();
    let mut fonts = egui::FontDefinitions::empty();
    let name = "Atkinson Hyperlegible Next";
    fonts.font_data.insert(
        name.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(
            fossps_render::Font::bundled_file(),
        )),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.insert(family, vec![name.to_owned()]);
    }
    context.set_fonts(fonts);
    context.set_theme(egui::Theme::Dark);
    context
}

/// What the person asked for in the panels this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Asked {
    /// Fly the camera to see something with this middle that reaches this
    /// far from it, in metres.
    pub(crate) fly_to: Option<([f32; 3], f32)>,
    /// The map was changed and needs saving.
    pub(crate) changed: bool,
}

/// What is chosen: something in the level file, by its place in the list of
/// them, or something placed in the map, by its ID.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pick {
    Level(usize),
    Placed(String),
}

/// How far something placed in the map reaches, for flying to it: about a
/// player's height.
const PLACED_REACH: f32 = 1.0;

/// What a placed entity goes by in the list: its name, or its type.
pub(crate) fn placed_name(entity: &map::Entity) -> &str {
    if entity.name.trim().is_empty() {
        entity.kind.as_str()
    } else {
        entity.name.as_str()
    }
}

/// The panels and what they show.
pub(crate) struct LevelPanels {
    entities: Vec<Entity>,
    /// What is chosen, and shown in the inspector.
    selected: Option<Pick>,
    /// The chosen placed entity's type as it is being typed: the entity
    /// keeps its last whole type while the field is empty.
    type_text: String,
    /// The ID of the entity `type_text` was last filled in from.
    typed_for: String,
    /// The type the next thing placed gets: the last one placed or chosen.
    next_type: String,
    /// Whether a click on the ground in the picture places something.
    pub(crate) placing: bool,
    /// How many rows of the list were placed entities, last frame.
    placed_rows: usize,
    /// What the list is called: the level's file, or that it is a stand-in.
    title: &'static str,
    /// Said under the title when the game has no level of its own.
    hint: Option<&'static str>,
    /// What the loader skipped or changed in the level file.
    warnings: Vec<String>,
    /// Where each listed row was drawn last frame, for tests to click on.
    #[cfg(test)]
    pub(crate) rows: Vec<(usize, egui::Rect)>,
    /// Where the buttons and fields were drawn last frame, by what they do.
    #[cfg(test)]
    pub(crate) spots: Vec<(&'static str, egui::Rect)>,
}

impl LevelPanels {
    /// Panels for `scene`, which is the engine's stand-in if `stand_in`.
    pub(crate) fn new(scene: &Scene, stand_in: bool) -> Self {
        Self {
            entities: entities(scene),
            selected: None,
            type_text: String::new(),
            typed_for: String::new(),
            next_type: map::SPAWN.to_owned(),
            placing: false,
            placed_rows: 0,
            title: if stand_in {
                "Stand-in level"
            } else {
                crate::scaffold::LEVEL_FILE
            },
            hint: stand_in.then_some(
                "This game has no level.glb yet, so it is played on the engine's stand-in. \
                 Export one from Blender as glTF Binary into the game's folder.",
            ),
            warnings: scene.warnings.clone(),
            #[cfg(test)]
            rows: Vec::new(),
            #[cfg(test)]
            spots: Vec::new(),
        }
    }

    /// Notes where a button or field was drawn, for tests to click on.
    #[cfg(test)]
    fn mark(&mut self, what: &'static str, response: &egui::Response) {
        self.spots.push((what, response.rect));
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)]
    fn mark(&mut self, _what: &'static str, _response: &egui::Response) {}

    /// Where the button or field called `what` was drawn last frame.
    #[cfg(test)]
    pub(crate) fn spot(&self, what: &str) -> egui::Pos2 {
        self.spots
            .iter()
            .find(|(name, _)| *name == what)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("nothing called {what:?} was drawn"))
    }

    /// The ID of the placed entity that is chosen, if one is.
    pub(crate) fn chosen_placed(&self) -> Option<&str> {
        match &self.selected {
            Some(Pick::Placed(id)) => Some(id.as_str()),
            _ => None,
        }
    }

    /// Chooses the placed entity with this ID, as a click on its row does.
    pub(crate) fn choose_placed(&mut self, id: &str) {
        if self.chosen_placed() != Some(id) {
            self.selected = Some(Pick::Placed(id.to_owned()));
        }
    }

    /// Places one more of what was placed or chosen last in `map`, at
    /// `spot` and turned `facing` degrees to the left, and chooses it. Says
    /// whether it could: a map can be full.
    pub(crate) fn place(&mut self, map: &mut Map, spot: [f32; 3], facing: f32) -> bool {
        let Some(id) = map.place(&self.next_type, spot, facing) else {
            return false;
        };
        self.selected = Some(Pick::Placed(id.to_owned()));
        true
    }

    /// Everything in the level file that is listed.
    #[cfg(test)]
    pub(crate) fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Which row of the list is chosen: placed entities come first.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<usize> {
        match &self.selected {
            Some(Pick::Level(index)) => Some(self.placed_rows + index),
            Some(Pick::Placed(_)) | None => None,
        }
    }

    /// Draws the panels into `ui`, which covers the window, with `status`
    /// (the camera's speed and place) in the bottom corner.
    ///
    /// `map` is what is placed in the level, which the panels change;
    /// `saved` says how saving it went, `spot` is where the next thing
    /// placed goes and how far it is turned to the left, in degrees, and
    /// `snap` is what things snap to, which a button changes.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        status: &str,
        map: &mut Map,
        saved: &str,
        spot: ([f32; 3], f32),
        snap: &mut Snap,
    ) -> Asked {
        let mut asked = Asked::default();
        // Something else may have saved the map without what was chosen.
        if let Some(Pick::Placed(id)) = &self.selected
            && map.get(id).is_none()
        {
            self.selected = None;
        }
        #[cfg(test)]
        self.spots.clear();
        egui::Panel::bottom(egui::Id::new("fossps how to fly")).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(KEYS);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(status);
                });
            });
            ui.label(EDIT_KEYS);
            ui.weak(GAMEPAD);
            ui.weak(EDIT_GAMEPAD);
        });
        egui::Panel::left(egui::Id::new("fossps level list"))
            .resizable(true)
            .default_size(LIST_WIDTH)
            .show(ui, |ui| self.list(ui, &mut asked, map, saved, spot, snap));
        egui::Panel::right(egui::Id::new("fossps inspector"))
            .resizable(true)
            .default_size(INSPECTOR_WIDTH)
            .show(ui, |ui| self.inspector(ui, &mut asked, map));
        asked
    }

    /// The list of everything placed in the map and everything in the
    /// level. A click chooses one and a double click flies to it.
    fn list(
        &mut self,
        ui: &mut egui::Ui,
        asked: &mut Asked,
        map: &mut Map,
        saved: &str,
        spot: ([f32; 3], f32),
        snap: &mut Snap,
    ) {
        ui.heading(self.title);
        ui.weak(count_words(&self.entities));
        if let Some(hint) = self.hint {
            ui.label(hint);
        }
        ui.separator();
        ui.strong(MAP_FILE);
        ui.weak(if map.is_empty() {
            "Nothing placed yet".to_owned()
        } else {
            map.summary()
        });
        let place = ui
            .button(format!("Place a {} here", self.next_type))
            .on_hover_text(
                "Puts one on the ground where the camera looks, as P and A on a gamepad do. \
                 Change what it is on the right; the next one placed is the same.",
            );
        self.mark("place", &place);
        if place.clicked() {
            asked.changed |= self.place(map, spot.0, spot.1);
        }
        let placing = ui
            .checkbox(&mut self.placing, "Click in the picture to place")
            .on_hover_text(
                "While this is on, a click on the ground puts one there. Esc turns it off.",
            );
        self.mark("placing", &placing);
        let snapping = ui.button(snap.words()).on_hover_text(
            "What things land on when they are placed, moved and turned. A click, G, or Y \
             on a gamepad changes it; holding Ctrl while dragging goes without.",
        );
        self.mark("snap", &snapping);
        if snapping.clicked() {
            *snap = snap.next();
        }
        if !saved.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(saved).weak()).wrap());
        }
        ui.separator();
        #[cfg(test)]
        self.rows.clear();
        let row_height = ui.spacing().interact_size.y;
        let placed = map.entities().len();
        self.placed_rows = placed;
        let total = placed + self.entities.len();
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_height,
            total,
            |ui, rows| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    for row in rows {
                        // Placed entities first: they are what is worked on.
                        let (name, word, pick, middle, reach) =
                            if let Some(entity) = map.entities().get(row) {
                                (
                                    placed_name(entity),
                                    entity.kind.as_str(),
                                    Pick::Placed(entity.id.clone()),
                                    entity.position,
                                    PLACED_REACH,
                                )
                            } else if let Some(entity) = self.entities.get(row - placed) {
                                (
                                    entity.name.as_str(),
                                    entity.kind.word(),
                                    Pick::Level(row - placed),
                                    entity.middle,
                                    entity.reach,
                                )
                            } else {
                                continue;
                            };
                        let chosen = self.selected.as_ref() == Some(&pick);
                        let button = egui::Button::selectable(chosen, name)
                            .right_text(egui::RichText::new(word).weak())
                            .truncate();
                        let response = ui.add(button);
                        #[cfg(test)]
                        self.rows.push((row, response.rect));
                        if response.clicked() {
                            self.selected = Some(pick);
                        }
                        if response.double_clicked() {
                            asked.fly_to = Some((middle, reach));
                        }
                    }
                });
            },
        );
    }

    /// What can be changed about the placed entity with this ID.
    fn placed_inspector(&mut self, ui: &mut egui::Ui, asked: &mut Asked, map: &mut Map, id: &str) {
        let Some(entity) = map.get_mut(id) else {
            return;
        };
        if self.typed_for != id {
            self.type_text.clone_from(&entity.kind);
            id.clone_into(&mut self.typed_for);
        }
        ui.heading(placed_name(entity));
        ui.weak(format!("placed in {MAP_FILE}"));
        ui.separator();
        let mut marks: Vec<(&'static str, egui::Response)> = Vec::new();
        let mut changed = false;
        egui::Grid::new(egui::Id::new("fossps placed"))
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.weak("Type");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.type_text)
                        .char_limit(map::LONGEST_WORD)
                        .desired_width(f32::INFINITY),
                );
                if field.changed() {
                    // Kept as it was while the field is empty or mid-word.
                    if map::is_type(&self.type_text) {
                        entity.kind.clone_from(&self.type_text);
                        self.next_type.clone_from(&self.type_text);
                        changed = true;
                    }
                } else if !field.has_focus() && self.type_text != entity.kind {
                    self.type_text.clone_from(&entity.kind);
                }
                marks.push(("type", field));
                ui.end_row();

                ui.weak("Name");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut entity.name)
                        .char_limit(map::LONGEST_TEXT)
                        .hint_text("none")
                        .desired_width(f32::INFINITY),
                );
                changed |= field.changed();
                marks.push(("name", field));
                ui.end_row();

                ui.weak("Position");
                ui.horizontal(|ui| {
                    for (axis, what) in ["x", "y", "z"].into_iter().enumerate() {
                        let field = ui.add(
                            egui::DragValue::new(&mut entity.position[axis])
                                .speed(0.05)
                                .range(-map::FURTHEST..=map::FURTHEST)
                                .max_decimals(3)
                                .suffix(" m"),
                        );
                        changed |= field.changed();
                        marks.push((what, field));
                    }
                });
                ui.end_row();

                ui.weak("Turned");
                let field = ui.add(
                    egui::DragValue::new(&mut entity.facing)
                        .speed(1.0)
                        .range(-180.0..=180.0)
                        .max_decimals(2)
                        .suffix("° to the left"),
                );
                changed |= field.changed();
                marks.push(("facing", field));
                ui.end_row();

                ui.weak("ID");
                ui.add(egui::Label::new(entity.id.as_str()).wrap())
                    .on_hover_text("Kept for good, so scripts can refer to it.");
                ui.end_row();
            });
        if !entity.properties.is_empty() {
            ui.separator();
            ui.weak("Properties");
            for (name, value) in &entity.properties {
                ui.add(
                    egui::Label::new(egui::RichText::new(format!("{name} = {value}")).monospace())
                        .wrap(),
                );
            }
        }
        ui.separator();
        let (middle, own_type) = (entity.position, entity.kind.clone());
        let mut remove = false;
        ui.horizontal(|ui| {
            if ui.button("Fly to it").clicked() {
                asked.fly_to = Some((middle, PLACED_REACH));
            }
            let button = ui
                .button("Remove")
                .on_hover_text("Takes it out of the map. There is no undo yet.");
            remove = button.clicked();
            marks.push(("remove", button));
        });
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "Changes are saved in {MAP_FILE} as you go, and a game that is running \
                     picks them up. Properties are added in that file for now, under a \
                     line saying [entity.properties]; a save there shows up here, unless a change made here is waiting to be saved."
                ))
                .weak(),
            )
            .wrap(),
        );
        for (what, response) in &marks {
            self.mark(what, response);
        }
        // The next one placed is the same sort as the one last looked at.
        if self.next_type != own_type {
            self.next_type = own_type;
        }
        if remove {
            map.remove(id);
            self.selected = None;
            changed = true;
        }
        asked.changed |= changed;
    }

    /// What is known about the chosen entity, or about the level if none is.
    fn inspector(&mut self, ui: &mut egui::Ui, asked: &mut Asked, map: &mut Map) {
        if let Some(Pick::Placed(id)) = self.selected.clone() {
            self.placed_inspector(ui, asked, map, &id);
            return;
        }
        let chosen = match &self.selected {
            Some(Pick::Level(index)) => self.entities.get(*index),
            _ => None,
        };
        let Some(entity) = chosen else {
            ui.heading("Nothing chosen");
            ui.label("Click a name in the list on the left to see it here.");
            if !self.warnings.is_empty() {
                ui.separator();
                ui.strong("Reading the level file");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for warning in &self.warnings {
                        ui.label(warning.as_str());
                    }
                });
            }
            return;
        };
        ui.heading(entity.name.as_str());
        ui.weak(entity.kind.word());
        ui.separator();
        egui::Grid::new(egui::Id::new("fossps facts"))
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for (what, value) in &entity.facts {
                    ui.weak(*what);
                    ui.add(egui::Label::new(value.as_str()).wrap());
                    ui.end_row();
                }
                if !entity.colors.is_empty() {
                    ui.weak(if entity.colors.len() == 1 {
                        "Colour"
                    } else {
                        "Colours"
                    });
                    ui.horizontal(|ui| {
                        for [red, green, blue] in &entity.colors {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                egui::Color32::from_rgb(*red, *green, *blue),
                            );
                        }
                    });
                    ui.end_row();
                }
            });
        if let Some(properties) = &entity.properties {
            ui.separator();
            ui.weak("Custom properties");
            ui.add(egui::Label::new(egui::RichText::new(properties.as_str()).monospace()).wrap());
        }
        ui.separator();
        if ui.button("Fly to it").clicked() {
            asked.fly_to = Some((entity.middle, entity.reach));
        }
        ui.weak("Or double-click its name in the list.");
        ui.add_space(8.0);
        ui.weak(
            "These values come from the level file, which the editor never writes: \
             change them in the 3D tool and export the level again.",
        );
    }
}

/// How many of each sort there are: "12 objects, 2 markers, 1 light".
fn count_words(entities: &[Entity]) -> String {
    let mut words = String::new();
    for kind in [Kind::Object, Kind::Marker, Kind::Light] {
        let count = entities.iter().filter(|entity| entity.kind == kind).count();
        if count == 0 {
            continue;
        }
        if !words.is_empty() {
            words.push_str(", ");
        }
        // Writing to a `String` cannot fail.
        let _ = write!(
            words,
            "{count} {}{}",
            kind.word(),
            if count == 1 { "" } else { "s" }
        );
    }
    if words.is_empty() {
        words.push_str("Nothing in it yet");
    }
    words
}
