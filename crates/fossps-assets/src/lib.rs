//! Reads levels and models for FOSSPS.
//!
//! Levels and models are [glTF 2.0](https://www.khronos.org/gltf/) files
//! (`.glb` or `.gltf`), which Blender and most other 3D tools export.
//! [`Scene::load`] turns one into plain data: shapes, where they stand,
//! materials and pictures, and for rigged models their skeletons
//! ([`Skin`]) and animation clips ([`Animation`]). Nothing here talks to a graphics card, so a
//! dedicated server reads the same files for collision.
//!
//! ```no_run
//! # fn main() -> Result<(), fossps_assets::AssetError> {
//! let level = fossps_assets::Scene::load("level.glb")?;
//! println!("{} shapes placed", level.instances.len());
//! # Ok(())
//! # }
//! ```
//!
//! Directions and sizes are glTF's: +Y is up, -Z is forward, +X is right,
//! and one unit is one metre.
//!
//! The `textures` feature (on by default) decodes PNG and JPEG pictures.
//! Without it no image decoder is built and scenes load without textures.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
mod files;
mod load;
mod material;
mod math;
mod mesh;
mod rig;
mod scene;
#[cfg(test)]
mod tests;

pub use error::AssetError;
pub use load::LoadOptions;
pub use scene::{
    AlphaMode, Animation, Bounds, Filter, Image, Instance, Interpolation, Joint, Keys, Light,
    LightKind, MAX_JOINTS, Marker, Material, Mesh, MeshPart, Node, Pose, Sampler, Scene, Skin,
    Texture, TextureRef, TextureTransform, Track, Transform, Wrap,
};
