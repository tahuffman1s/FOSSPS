//! Reads what makes a model move: its skins (skeletons) and animation clips.

use std::collections::HashSet;

use gltf::accessor::{DataType, Dimensions};
use gltf::animation::util::ReadOutputs;
use gltf::animation::{Interpolation as GltfInterpolation, Property};

use crate::error::AssetError;
use crate::load::number;
use crate::math::IDENTITY;
use crate::scene::{
    Animation, Interpolation, Joint, Keys, MAX_JOINTS, Pose, Scene, Skin, Track, Transform,
};

/// The most memory one file's animation tracks may take once unpacked.
///
/// Many tracks may share one list of keys, so a small file could otherwise
/// ask for any amount of memory. A few minutes of clips for a hundred-bone
/// character, keyed thirty times a second, is about a tenth of this.
pub(crate) const ANIMATION_BYTES: usize = 256 * 1024 * 1024;

/// Marks a glTF node that is not in the scene being read.
pub(crate) const NOT_PLACED: u32 = u32::MAX;

/// What reading skins and animations needs to know about the file.
pub(crate) struct Rig<'a> {
    /// The file's name, for messages.
    pub(crate) file: &'a str,
    /// The data of each glTF buffer.
    pub(crate) buffers: &'a [Vec<u8>],
    /// For each glTF node, its place in [`Scene::nodes`], or [`NOT_PLACED`].
    pub(crate) node_of: &'a [u32],
}

/// How a thing is called in messages: its name in quotes, or its number.
pub(crate) fn shown(name: Option<&str>, index: usize) -> String {
    name.map_or_else(
        || format!("number {}", index + 1),
        |name| format!("\"{name}\""),
    )
}

/// `q` scaled to length one, or `None` if it has no direction.
///
/// Uses only a square root and divisions, which every computer rounds alike.
fn unit(q: [f32; 4]) -> Option<[f32; 4]> {
    let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    (length.is_finite() && length > 0.0).then(|| q.map(|v| v / length))
}

/// Where `node` sits relative to its parent when nothing is animated.
pub(crate) fn rest(file: &str, node: &gltf::Node<'_>) -> Result<Pose, AssetError> {
    let (translation, rotation, scale) = node.transform().decomposed();
    if !translation.iter().chain(&scale).all(|v| v.is_finite()) {
        return Err(AssetError::damaged(
            file,
            format!(
                "object {} is placed with numbers too large to use",
                shown(node.name(), node.index())
            ),
        ));
    }
    Ok(Pose {
        translation,
        // An object squashed flat has no rotation to speak of.
        rotation: unit(rotation).unwrap_or(Pose::default().rotation),
        scale,
    })
}

impl Rig<'_> {
    fn data(&self, buffer: &gltf::Buffer<'_>) -> Option<&[u8]> {
        self.buffers.get(buffer.index()).map(Vec::as_slice)
    }

    /// The scene node a glTF node became, if it is in the scene.
    fn node(&self, node: &gltf::Node<'_>) -> Option<u32> {
        self.node_of
            .get(node.index())
            .copied()
            .filter(|&node| node != NOT_PLACED)
    }

    /// Gives each rigged node its skin. `skinned` pairs a scene node that
    /// places a mesh with the glTF skin it asks for.
    pub(crate) fn skins(
        &self,
        skinned: &[(u32, gltf::Skin<'_>)],
        skins_in_file: usize,
        scene: &mut Scene,
    ) -> Result<(), AssetError> {
        // A skin or mesh used by many objects is read and checked once.
        let mut read: Vec<Option<u32>> = vec![None; skins_in_file];
        let mut reach: Vec<Option<Option<u16>>> = vec![None; scene.meshes.len()];
        for (node, source) in skinned {
            let node = *node as usize;
            let Some(mesh_index) = scene
                .nodes
                .get(node)
                .and_then(|node| node.instance)
                .and_then(|instance| scene.instances.get(instance as usize))
                .map(|instance| instance.mesh as usize)
            else {
                continue;
            };
            let (Some(mesh), Some(known)) =
                (scene.meshes.get(mesh_index), reach.get_mut(mesh_index))
            else {
                continue;
            };
            let highest = *known.get_or_insert_with(|| {
                let highest = highest_joint(mesh);
                if highest.is_none() && !mesh.parts.is_empty() {
                    scene.warnings.push(format!(
                        "Mesh {} is attached to a skeleton but has a part with no bone weights, so it will not bend. \
                         In Blender, parent it to the armature with automatic weights, or weight-paint it.",
                        shown(mesh.name.as_deref(), mesh_index)
                    ));
                }
                highest
            });
            let Some(highest) = highest else {
                continue;
            };
            let Some(slot) = read.get_mut(source.index()) else {
                continue;
            };
            let skin = match *slot {
                Some(skin) => skin,
                None => {
                    scene.skins.push(self.skin(source)?);
                    *slot.insert(number(scene.skins.len() - 1))
                }
            };
            let joints = scene.skins.get(skin as usize).map_or(0, |s| s.joints.len());
            if usize::from(highest) >= joints {
                return Err(AssetError::damaged(
                    self.file,
                    format!(
                        "mesh {} is moved by bone number {}, but its skeleton {} has only {joints}",
                        shown(mesh.name.as_deref(), mesh_index),
                        u32::from(highest) + 1,
                        shown(source.name(), source.index()),
                    ),
                ));
            }
            if let Some(node) = scene.nodes.get_mut(node) {
                node.skin = Some(skin);
            }
        }
        Ok(())
    }

    /// Reads one skin: which nodes are its joints and where the mesh sat on them.
    fn skin(&self, source: &gltf::Skin<'_>) -> Result<Skin, AssetError> {
        let name = shown(source.name(), source.index());
        let count = source.joints().count();
        if count == 0 {
            return Err(AssetError::damaged(
                self.file,
                format!("the skeleton {name} has no bones"),
            ));
        }
        if count > MAX_JOINTS {
            return Err(AssetError::TooManyJoints {
                file: self.file.to_owned(),
                skin: name,
                joints: count,
            });
        }
        let mut inverse_binds: Vec<Transform> = match source.inverse_bind_matrices() {
            // Without the list, the mesh was rigged where the joints stand.
            None => vec![IDENTITY; count],
            Some(accessor) => {
                if accessor.dimensions() != Dimensions::Mat4
                    || accessor.data_type() != DataType::F32
                    || accessor.count() < count
                {
                    return Err(AssetError::damaged(
                        self.file,
                        format!(
                            "the skeleton {name} has {count} bones but stores their bind poses as {} {:?} {:?}",
                            accessor.count(),
                            accessor.data_type(),
                            accessor.dimensions()
                        ),
                    ));
                }
                source
                    .reader(|buffer| self.data(&buffer))
                    .read_inverse_bind_matrices()
                    .map(|matrices| matrices.take(count).collect())
                    .unwrap_or_default()
            }
        };
        if inverse_binds.len() != count
            || !inverse_binds
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite())
        {
            return Err(AssetError::damaged(
                self.file,
                format!("the skeleton {name} has bind poses that cannot be read"),
            ));
        }
        let joints = source
            .joints()
            .zip(inverse_binds.drain(..))
            .map(|(joint, inverse_bind)| {
                let node = self.node(&joint).ok_or_else(|| {
                    AssetError::damaged(
                        self.file,
                        format!(
                            "the skeleton {name} uses the bone {}, which is not in the scene",
                            shown(joint.name(), joint.index())
                        ),
                    )
                })?;
                Ok(Joint { node, inverse_bind })
            })
            .collect::<Result<_, AssetError>>()?;
        Ok(Skin {
            name: source.name().map(str::to_owned),
            joints,
        })
    }

    /// Reads every animation clip.
    pub(crate) fn animations(
        &self,
        document: &gltf::Document,
        scene: &mut Scene,
    ) -> Result<(), AssetError> {
        let mut budget = ANIMATION_BYTES;
        let mut shape_keys = false;
        for source in document.animations() {
            let name = shown(source.name(), source.index());
            let mut tracks = Vec::new();
            // glTF allows one track for each thing moved; a second is dropped.
            let mut moved = HashSet::new();
            for channel in source.channels() {
                let target = channel.target();
                // What is not in the scene is not shown, so not animated either.
                let Some(node) = self.node(&target.node()) else {
                    continue;
                };
                let property = target.property();
                if property == Property::MorphTargetWeights {
                    shape_keys = true;
                    continue;
                }
                if !moved.insert((node, property as u8)) {
                    continue;
                }
                tracks.push(self.track(&name, &channel, node, property, &mut budget)?);
            }
            let duration = tracks
                .iter()
                .filter_map(|track: &Track| track.times.last().copied())
                .fold(0.0, f32::max);
            scene.animations.push(Animation {
                name: source.name().map(str::to_owned),
                duration,
                tracks,
            });
        }
        if shape_keys {
            scene.warnings.push(
                "The file animates shape keys (morph targets), which FOSSPS does not read; \
                 those parts of its animations are left out. Move the mesh with bones instead."
                    .to_owned(),
            );
        }
        Ok(())
    }

    /// Reads one track of the animation called `name`.
    fn track(
        &self,
        name: &str,
        channel: &gltf::animation::Channel<'_>,
        node: u32,
        property: Property,
        budget: &mut usize,
    ) -> Result<Track, AssetError> {
        let damaged = |problem: String| {
            AssetError::damaged(self.file, format!("the animation {name} {problem}"))
        };
        let sampler = channel.sampler();
        let (input, output) = (sampler.input(), sampler.output());
        let count = input.count();
        if input.dimensions() != Dimensions::Scalar
            || input.data_type() != DataType::F32
            || count == 0
        {
            return Err(damaged(format!(
                "stores its key times as {count} {:?} {:?}, which glTF does not allow",
                input.data_type(),
                input.dimensions()
            )));
        }
        let (interpolation, per_key) = match sampler.interpolation() {
            GltfInterpolation::Step => (Interpolation::Step, 1),
            GltfInterpolation::Linear => (Interpolation::Linear, 1),
            GltfInterpolation::CubicSpline => (Interpolation::CubicSpline, 3),
        };
        if interpolation == Interpolation::CubicSpline && count < 2 {
            return Err(damaged(
                "has a smooth curve through a single key, which needs at least two".to_owned(),
            ));
        }
        // The glTF reader trusts the list to hold what the track moves.
        let (dimensions, types, size): (_, &[DataType], usize) = match property {
            Property::Rotation => (
                Dimensions::Vec4,
                &[
                    DataType::F32,
                    DataType::I8,
                    DataType::U8,
                    DataType::I16,
                    DataType::U16,
                ],
                16,
            ),
            _ => (Dimensions::Vec3, &[DataType::F32], 12),
        };
        if output.dimensions() != dimensions || !types.contains(&output.data_type()) {
            return Err(damaged(format!(
                "stores a {property:?} track as {:?} {:?}, which glTF does not allow",
                output.data_type(),
                output.dimensions()
            )));
        }
        let values = output.count();
        if count.checked_mul(per_key) != Some(values) {
            return Err(damaged(format!(
                "has a track with {count} key times but {values} values, where it needs {per_key} for each time"
            )));
        }
        // `values` items fit in the file's data, so this cannot overflow.
        let bytes = count * 4 + values * size;
        *budget = budget
            .checked_sub(bytes)
            .ok_or_else(|| AssetError::TooMuchAnimation {
                file: self.file.to_owned(),
            })?;

        let unreadable = || damaged("has a track that cannot be read".to_owned());
        let reader = channel.reader(|buffer| self.data(&buffer));
        let times: Vec<f32> = reader.read_inputs().ok_or_else(unreadable)?.collect();
        let in_order = times.first().is_some_and(|&first| first >= 0.0)
            && times.iter().all(|time| time.is_finite())
            && times.windows(2).all(|pair| pair[0] < pair[1]);
        if times.len() != count || !in_order {
            return Err(damaged(
                "has key times that do not run forward from zero".to_owned(),
            ));
        }
        let keys = match reader.read_outputs().ok_or_else(unreadable)? {
            ReadOutputs::Translations(list) => Keys::Translation(list.collect()),
            ReadOutputs::Scales(list) => Keys::Scale(list.collect()),
            ReadOutputs::Rotations(list) => {
                let mut list: Vec<[f32; 4]> = list.into_f32().collect();
                // In a smooth curve only the middle of each three is a rotation.
                for value in list.iter_mut().skip(per_key / 2).step_by(per_key) {
                    *value = unit(*value).ok_or_else(|| {
                        damaged("has a rotation key that is all zeros or not a number".to_owned())
                    })?;
                }
                Keys::Rotation(list)
            }
            ReadOutputs::MorphTargetWeights(_) => return Err(unreadable()),
        };
        let (length, finite) = match &keys {
            Keys::Translation(list) | Keys::Scale(list) => {
                (list.len(), list.iter().flatten().all(|v| v.is_finite()))
            }
            Keys::Rotation(list) => (list.len(), list.iter().flatten().all(|v| v.is_finite())),
        };
        if length != values || !finite {
            return Err(damaged("has a key that is not a number".to_owned()));
        }
        Ok(Track {
            node,
            interpolation,
            times,
            keys,
        })
    }
}

/// The highest joint number any point of `mesh` uses, or `None` if a part
/// of it has no bone weights (or it has no parts).
fn highest_joint(mesh: &crate::scene::Mesh) -> Option<u16> {
    mesh.parts
        .iter()
        .map(|part| {
            part.joints
                .as_ref()
                .map(|joints| joints.iter().flatten().copied().max().unwrap_or(0))
        })
        .try_fold(None, |highest: Option<u16>, part| {
            part.map(|part| Some(highest.map_or(part, |highest| highest.max(part))))
        })
        .flatten()
}
