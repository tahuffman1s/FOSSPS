//! Compiles the WGSL shaders in `shaders/` to SPIR-V with naga.
//!
//! Doing this in pure Rust means building FOSSPS needs no Vulkan SDK or
//! system shader compiler.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::{env, fs};

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};

/// Shader files to compile, by name without the `.wgsl` extension.
const SHADERS: &[&str] = &["triangle"];

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(env::var("OUT_DIR")?);
    for name in SHADERS {
        let source = Path::new("shaders").join(format!("{name}.wgsl"));
        println!("cargo::rerun-if-changed={}", source.display());
        let words = compile(&source)
            .map_err(|error| format!("Could not compile {}: {error}", source.display()))?;
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        fs::write(out.join(format!("{name}.spv")), bytes)?;
    }
    Ok(())
}

/// Turns one WGSL file into SPIR-V holding all of its entry points.
fn compile(path: &Path) -> Result<Vec<u32>, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let module =
        naga::front::wgsl::parse_str(&text).map_err(|error| error.emit_to_string(&text))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| error.emit_to_string(&text))?;
    let options = spv::Options {
        // Vulkan 1.3 accepts SPIR-V up to 1.6; 1.3 is plenty and widely tested.
        lang_version: (1, 3),
        ..spv::Options::default()
    };
    Ok(spv::write_vec(&module, &info, &options, None)?)
}
