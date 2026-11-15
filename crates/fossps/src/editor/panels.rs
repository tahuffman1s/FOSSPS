//! The editor's panels: what is in the level on the left, what is known
//! about the chosen thing on the right, and how to fly along the bottom.
//!
//! They are made with `egui` and are for the mouse. Nothing in them can be
//! changed yet; they show what the level file holds.

use std::fmt::Write as _;

use fossps_assets::{Bounds, LightKind, Scene, Transform};

/// How wide the two side panels start out, in `egui` points.
const LIST_WIDTH: f32 = 260.0;
const INSPECTOR_WIDTH: f32 = 300.0;

/// How to fly, said along the bottom of the window.
const KEYS: &str = "Look: hold the right mouse button    Fly: W A S D    Up, down: E, Q    \
                    Faster: hold Shift    Speed: mouse wheel";
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Asked {
    /// Fly the camera to this one of the entities.
    pub(crate) fly_to: Option<usize>,
}

/// The panels and what they show.
pub(crate) struct LevelPanels {
    entities: Vec<Entity>,
    /// Which of them is chosen, and shown in the inspector.
    selected: Option<usize>,
    /// What the list is called: the level's file, or that it is a stand-in.
    title: &'static str,
    /// Said under the title when the game has no level of its own.
    hint: Option<&'static str>,
    /// What the loader skipped or changed in the level file.
    warnings: Vec<String>,
    /// Where each listed row was drawn last frame, for tests to click on.
    #[cfg(test)]
    pub(crate) rows: Vec<(usize, egui::Rect)>,
}

impl LevelPanels {
    /// Panels for `scene`, which is the engine's stand-in if `stand_in`.
    pub(crate) fn new(scene: &Scene, stand_in: bool) -> Self {
        Self {
            entities: entities(scene),
            selected: None,
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
        }
    }

    /// Everything listed.
    pub(crate) fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Which entity is chosen.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Draws the panels into `ui`, which covers the window, with `status`
    /// (the camera's speed and place) in the bottom corner.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, status: &str) -> Asked {
        let mut asked = Asked::default();
        egui::Panel::bottom(egui::Id::new("fossps how to fly")).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(KEYS);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(status);
                });
            });
            ui.weak(GAMEPAD);
        });
        egui::Panel::left(egui::Id::new("fossps level list"))
            .resizable(true)
            .default_size(LIST_WIDTH)
            .show(ui, |ui| self.list(ui, &mut asked));
        egui::Panel::right(egui::Id::new("fossps inspector"))
            .resizable(true)
            .default_size(INSPECTOR_WIDTH)
            .show(ui, |ui| self.inspector(ui, &mut asked));
        asked
    }

    /// The list of everything in the level. A click chooses one and a
    /// double click flies to it.
    fn list(&mut self, ui: &mut egui::Ui, asked: &mut Asked) {
        ui.heading(self.title);
        ui.weak(count_words(&self.entities));
        if let Some(hint) = self.hint {
            ui.label(hint);
        }
        ui.separator();
        #[cfg(test)]
        self.rows.clear();
        let row_height = ui.spacing().interact_size.y;
        let total = self.entities.len();
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_height,
            total,
            |ui, rows| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    for row in rows {
                        let Some(entity) = self.entities.get(row) else {
                            continue;
                        };
                        let chosen = self.selected == Some(row);
                        let button = egui::Button::selectable(chosen, entity.name.as_str())
                            .right_text(egui::RichText::new(entity.kind.word()).weak())
                            .truncate();
                        let response = ui.add(button);
                        #[cfg(test)]
                        self.rows.push((row, response.rect));
                        if response.clicked() {
                            self.selected = Some(row);
                        }
                        if response.double_clicked() {
                            asked.fly_to = Some(row);
                        }
                    }
                });
            },
        );
    }

    /// What is known about the chosen entity, or about the level if none is.
    fn inspector(&mut self, ui: &mut egui::Ui, asked: &mut Asked) {
        let Some((index, entity)) = self
            .selected
            .and_then(|index| Some((index, self.entities.get(index)?)))
        else {
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
            asked.fly_to = Some(index);
        }
        ui.weak("Or double-click its name in the list.");
        ui.add_space(8.0);
        ui.weak("These values come from the level file and cannot be changed here yet.");
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
