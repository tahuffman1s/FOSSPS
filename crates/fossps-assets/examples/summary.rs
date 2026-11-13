//! Prints what FOSSPS reads from a level or model file.
//!
//! ```sh
//! cargo run -p fossps-assets --example summary -- level.glb
//! ```

use std::process::ExitCode;

use fossps_assets::Scene;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("Say which file to read: summary <level.glb>");
        return ExitCode::FAILURE;
    };
    let scene = match Scene::load(&path) {
        Ok(scene) => scene,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let parts = scene.meshes.iter().flat_map(|mesh| &mesh.parts);
    let triangles: usize = parts.clone().map(|part| part.indices.len() / 3).sum();
    let with_tangents = parts.filter(|part| part.tangents.is_some()).count();
    println!(
        "{} meshes ({triangles} triangles, {with_tangents} parts with tangents) placed {} times",
        scene.meshes.len(),
        scene.instances.len()
    );
    println!(
        "{} materials, {} textures, {} images, {} lights, {} markers",
        scene.materials.len(),
        scene.textures.len(),
        scene.images.len(),
        scene.lights.len(),
        scene.markers.len()
    );
    for image in &scene.images {
        println!(
            "  image {}: {} x {}",
            image.name.as_deref().unwrap_or("(unnamed)"),
            image.width,
            image.height
        );
    }
    for warning in &scene.warnings {
        println!("warning: {warning}");
    }
    ExitCode::SUCCESS
}
