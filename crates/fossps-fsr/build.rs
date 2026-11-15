//! Builds AMD's FSR 3.1 upscaler from the source in `vendor/`.
//!
//! AMD's own build only runs on Windows: its shaders are compiled by a
//! Windows program and its C++ leans on MSVC. This script does both jobs with
//! crates, so `cargo build` is all it takes on Linux, Windows and macOS:
//!
//! 1. the GLSL shaders are compiled to SPIR-V with glslang, and a table of
//!    them, with the names and slots of what each one reads and writes, is
//!    written as C++ for the SDK to look them up in;
//! 2. the SDK's C++, that table and `shim/shim.cpp` are compiled with `cc`.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use glslang::include::{IncludeHandler, IncludeResult, IncludeType};
use glslang::{
    Compiler, CompilerOptions, ShaderInput, ShaderMessage, ShaderSource, ShaderStage,
    SourceLanguage, SpirvVersion, Target, VulkanVersion,
};

const SDK: &str = "vendor/FidelityFX-SDK";

/// Each pass the SDK can ask for, and the shader file that draws it.
const PASSES: &[(&str, &str)] = &[
    ("FFX_FSR3UPSCALER_PASS_PREPARE_INPUTS", "prepare_inputs"),
    ("FFX_FSR3UPSCALER_PASS_LUMA_PYRAMID", "luma_pyramid"),
    (
        "FFX_FSR3UPSCALER_PASS_SHADING_CHANGE_PYRAMID",
        "shading_change_pyramid",
    ),
    ("FFX_FSR3UPSCALER_PASS_SHADING_CHANGE", "shading_change"),
    (
        "FFX_FSR3UPSCALER_PASS_PREPARE_REACTIVITY",
        "prepare_reactivity",
    ),
    ("FFX_FSR3UPSCALER_PASS_LUMA_INSTABILITY", "luma_instability"),
    ("FFX_FSR3UPSCALER_PASS_ACCUMULATE", "accumulate"),
    ("FFX_FSR3UPSCALER_PASS_ACCUMULATE_SHARPEN", "accumulate"),
    ("FFX_FSR3UPSCALER_PASS_RCAS", "rcas"),
    ("FFX_FSR3UPSCALER_PASS_DEBUG_VIEW", "debug_view"),
    (
        "FFX_FSR3UPSCALER_PASS_GENERATE_REACTIVE",
        "autogen_reactive",
    ),
];

/// The options every shader is compiled with.
///
/// Most are the ones AMD's build passes. The last four are choices the SDK
/// compiles both ways and FOSSPS makes once: the picture is high
/// dynamic range, motion vectors are the size of the picture and not
/// jittered, and depth is 1 at the camera and 0 far away. `shim/blobs.cpp`
/// and `shim/shim.cpp` hold the same choices.
const DEFINES: &[(&str, &str)] = &[
    ("FFX_GPU", "1"),
    ("FFX_GLSL", "1"),
    (
        "FFX_FSR3UPSCALER_OPTION_UPSAMPLE_SAMPLERS_USE_DATA_HALF",
        "0",
    ),
    (
        "FFX_FSR3UPSCALER_OPTION_ACCUMULATE_SAMPLERS_USE_DATA_HALF",
        "0",
    ),
    (
        "FFX_FSR3UPSCALER_OPTION_REPROJECT_SAMPLERS_USE_DATA_HALF",
        "1",
    ),
    (
        "FFX_FSR3UPSCALER_OPTION_POSTPROCESSLOCKSTATUS_SAMPLERS_USE_DATA_HALF",
        "0",
    ),
    ("FFX_FSR3UPSCALER_OPTION_UPSAMPLE_USE_LANCZOS_TYPE", "2"),
    // The last frames are resampled with the Lanczos curve worked out in the
    // shader. AMD's other variant reads the curve from a small texture, on
    // cards with 32- to 64-wide subgroups; built this way it made a still
    // picture drift and blur on an RX 7800 XT (RADV), so it is not built.
    ("FFX_FSR3UPSCALER_OPTION_REPROJECT_USE_LANCZOS_TYPE", "0"),
    // Not AMD's choice: without this the two pyramid passes use subgroup
    // "quad" operations, which Vulkan does not promise in compute shaders.
    // The plain path reads four pixels itself and runs on every card.
    ("FFX_SPD_NO_WAVE_OPERATIONS", "1"),
    ("FFX_FSR3UPSCALER_OPTION_HDR_COLOR_INPUT", "1"),
    ("FFX_FSR3UPSCALER_OPTION_LOW_RESOLUTION_MOTION_VECTORS", "1"),
    ("FFX_FSR3UPSCALER_OPTION_JITTERED_MOTION_VECTORS", "0"),
    ("FFX_FSR3UPSCALER_OPTION_INVERTED_DEPTH", "1"),
];

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=shim");
    println!("cargo::rerun-if-changed=vendor");
    let out = PathBuf::from(env::var("OUT_DIR")?);
    let sdk = Path::new(SDK);

    fs::write(out.join("fossps_fsr_blobs.inc"), shader_table(sdk)?)?;

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .warnings(false)
        .include(sdk.join("include"))
        .include(sdk.join("src/shared"))
        .include(sdk.join("src/backends/shared"))
        .include(sdk.join("src/components"))
        .include("vendor/Vulkan-Headers/include")
        .include("shim")
        .include(&out)
        .define("FFX_FSR3UPSCALER", None)
        // The context holds names as wchar_t, which is 4 bytes outside
        // Windows, so AMD's size is too small there. Twice as much fits.
        .define("FFX_SDK_DEFAULT_CONTEXT_SIZE", "(1024*256)")
        .file(sdk.join("src/backends/vk/ffx_vk.cpp"))
        .file(sdk.join("src/components/fsr3upscaler/ffx_fsr3upscaler.cpp"))
        .file(sdk.join("src/shared/ffx_assert.cpp"))
        .file(sdk.join("src/shared/ffx_breadcrumbs_list.cpp"))
        .file(sdk.join("src/shared/ffx_message.cpp"))
        .file(sdk.join("src/shared/ffx_object_management.cpp"))
        .file("shim/blobs.cpp")
        .file("shim/shim.cpp");
    // `compat.h` goes in front of every file, so the SDK's files stay as AMD wrote them.
    if build.get_compiler().is_like_msvc() {
        build.flag("/FIcompat.h");
    } else {
        build
            .flag("-include")
            .flag("shim/compat.h")
            .define("FFX_GCC", None);
    }
    build.try_compile("fossps_fsr")?;
    Ok(())
}

/// Finds `#include`d files the way AMD's build does: next to the file that
/// asks, then in the SDK's shader folders.
struct Includes {
    folders: Vec<PathBuf>,
}

impl IncludeHandler for Includes {
    fn include(
        &mut self,
        _kind: IncludeType,
        header: &str,
        includer: &str,
        _depth: usize,
    ) -> Option<IncludeResult> {
        let beside = Path::new(includer).parent().map(Path::to_path_buf);
        beside
            .iter()
            .chain(&self.folders)
            .map(|folder| folder.join(header))
            .find(|path| path.is_file())
            .and_then(|path| {
                Some(IncludeResult {
                    data: corrected(fs::read_to_string(&path).ok()?),
                    name: path.to_string_lossy().into_owned(),
                })
            })
    }
}

/// AMD's GLSL says the luminance history image holds 8-bit numbers, but the
/// SDK creates it with 16-bit floats (as the HLSL has it). Vulkan leaves
/// writes through a mismatched declaration undefined, and the validation
/// layer says so on every frame, so the declaration is put right here
/// rather than in the vendored file.
fn corrected(text: String) -> String {
    text.replace(
        "FSR3UPSCALER_BIND_UAV_LUMA_HISTORY, rgba8)",
        "FSR3UPSCALER_BIND_UAV_LUMA_HISTORY, rgba16f)",
    )
}

/// Compiles every shader and returns the C++ table `shim/blobs.cpp` includes.
fn shader_table(sdk: &Path) -> Result<String, Box<dyn Error>> {
    let compiler = Compiler::acquire().ok_or("glslang could not be started")?;
    let shaders = sdk.join("src/backends/vk/shaders/fsr3upscaler");
    let gpu = sdk.join("include/FidelityFX/gpu");
    let folders = vec![gpu.join("fsr3upscaler"), gpu];

    let mut code = String::from("// Written by build.rs.\n\n");
    // Many choices of options compile to the same shader; each is stored once.
    let mut stored: HashMap<Vec<u32>, usize> = HashMap::new();
    let mut table = String::new();
    for (pass, file) in PASSES {
        let path = shaders.join(format!("ffx_fsr3upscaler_{file}_pass.glsl"));
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let source = ShaderSource::from(text);
        for sharpen in [false, true] {
            let words = compile(compiler, &source, &folders, sharpen)
                .map_err(|error| format!("Could not compile {}: {error}", path.display()))?;
            let next = stored.len();
            let index = if let Some(index) = stored.get(&words) {
                *index
            } else {
                write_blob(&mut code, next, &words)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                stored.insert(words, next);
                next
            };
            writeln!(table, "    {{ {pass}, {sharpen}, &g_blob{index} }},")?;
        }
    }
    write!(
        code,
        "static const FosspsFsrShader g_shaders[] = {{\n{table}}};\n"
    )?;
    Ok(code)
}

/// Compiles one shader with one choice of options to SPIR-V.
fn compile(
    compiler: &Compiler,
    source: &ShaderSource,
    folders: &[PathBuf],
    sharpen: bool,
) -> Result<Vec<u32>, Box<dyn Error>> {
    let flag = |on: bool| Some(if on { "1" } else { "0" });
    let mut defines: Vec<(&str, Option<&str>)> = DEFINES
        .iter()
        .map(|(name, value)| (*name, Some(*value)))
        .collect();
    defines.push(("FFX_FSR3UPSCALER_OPTION_APPLY_SHARPENING", flag(sharpen)));

    let mut includes = Includes {
        folders: folders.to_vec(),
    };
    let options = CompilerOptions {
        source_language: SourceLanguage::GLSL,
        // What AMD's build asks for (`--target-env vulkan1.2`).
        target: Target::Vulkan {
            version: VulkanVersion::Vulkan1_2,
            spirv_version: SpirvVersion::SPIRV1_5,
        },
        version_profile: None,
        messages: ShaderMessage::DEFAULT,
    };
    let input = ShaderInput::new(
        source,
        ShaderStage::Compute,
        &options,
        Some(&defines),
        Some(&mut includes),
    )?;
    let shader = compiler.create_shader(input)?;
    Ok(shader.compile_size_optimized()?)
}

/// What kind of thing a shader reads or writes, as the SDK sorts them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Constants,
    Texture,
    WritableTexture,
    Sampler,
}

/// One thing a shader reads or writes.
struct Binding {
    kind: Kind,
    name: String,
    slot: u32,
    count: u32,
    set: u32,
}

/// Appends one compiled shader, and the lists the SDK wants with it, as C++.
fn write_blob(code: &mut String, index: usize, words: &[u32]) -> Result<(), Box<dyn Error>> {
    let bindings = reflect(words)?;
    write!(code, "static const uint32_t g_words{index}[] = {{")?;
    for (position, word) in words.iter().enumerate() {
        if position % 12 == 0 {
            code.push_str("\n   ");
        }
        write!(code, " {word:#x},")?;
    }
    code.push_str("\n};\n");

    let mut counts = Vec::new();
    let mut fields = Vec::new();
    for (kind, short) in [
        (Kind::Constants, "cb"),
        (Kind::Texture, "srv"),
        (Kind::WritableTexture, "uav"),
        (Kind::Sampler, "sampler"),
    ] {
        let of_kind: Vec<&Binding> = bindings.iter().filter(|b| b.kind == kind).collect();
        counts.push(of_kind.len());
        if of_kind.is_empty() {
            fields.push("nullptr, nullptr, nullptr, nullptr".to_owned());
            continue;
        }
        let list = |values: Vec<String>| values.join(", ");
        writeln!(
            code,
            "static const char* g_{short}Names{index}[] = {{ {} }};",
            list(of_kind.iter().map(|b| format!("\"{}\"", b.name)).collect())
        )?;
        for (what, values) in [
            ("Slots", of_kind.iter().map(|b| b.slot).collect::<Vec<_>>()),
            ("Counts", of_kind.iter().map(|b| b.count).collect()),
            ("Sets", of_kind.iter().map(|b| b.set).collect()),
        ] {
            writeln!(
                code,
                "static const uint32_t g_{short}{what}{index}[] = {{ {} }};",
                list(values.iter().map(u32::to_string).collect())
            )?;
        }
        fields.push(format!(
            "g_{short}Names{index}, g_{short}Slots{index}, g_{short}Counts{index}, g_{short}Sets{index}"
        ));
    }
    // The order of `FfxShaderBlob`: the bytes; how many constant buffers,
    // textures, writable textures, buffers, writable buffers, samplers and
    // acceleration structures; then four lists for each of those seven.
    let none = "nullptr, nullptr, nullptr, nullptr";
    writeln!(
        code,
        "static const FfxShaderBlob g_blob{index} = {{\n    \
         reinterpret_cast<const uint8_t*>(g_words{index}), sizeof(g_words{index}),\n    \
         {}, {}, {}, 0, 0, {}, 0,\n    {},\n    {},\n    {},\n    {none},\n    {none},\n    {},\n    {none},\n}};\n",
        counts[0], counts[1], counts[2], counts[3], fields[0], fields[1], fields[2], fields[3],
    )?;
    Ok(())
}

/// Reads from SPIR-V what a shader reads and writes: names, slots and kinds.
///
/// AMD's tool gets this from SPIRV-Reflect, after an optimiser has removed
/// what a shader declares but never touches. glslang is built here without
/// that optimiser, so only variables some instruction uses are listed: the
/// SDK does not know the names of the others.
///
/// The upscaler's shaders only use uniform blocks, textures, storage images
/// and samplers, so walking the few instructions that describe those is
/// enough; anything else is an error rather than a guess.
fn reflect(words: &[u32]) -> Result<Vec<Binding>, Box<dyn Error>> {
    // SPIR-V instruction numbers.
    const OP_NAME: u32 = 5;
    const OP_TYPE_IMAGE: u32 = 25;
    const OP_TYPE_SAMPLER: u32 = 26;
    const OP_TYPE_ARRAY: u32 = 28;
    const OP_TYPE_STRUCT: u32 = 30;
    const OP_TYPE_POINTER: u32 = 32;
    const OP_CONSTANT: u32 = 43;
    const OP_FUNCTION_CALL: u32 = 57;
    const OP_VARIABLE: u32 = 59;
    const OP_IMAGE_TEXEL_POINTER: u32 = 60;
    const OP_LOAD: u32 = 61;
    const OP_STORE: u32 = 62;
    const OP_ACCESS_CHAIN: u32 = 65;
    const OP_IN_BOUNDS_ACCESS_CHAIN: u32 = 66;
    const OP_COPY_OBJECT: u32 = 83;
    const OP_DECORATE: u32 = 71;
    const DECORATION_BLOCK: u32 = 2;
    const DECORATION_BINDING: u32 = 33;
    const DECORATION_SET: u32 = 34;
    const STORAGE_UNIFORM_CONSTANT: u32 = 0;
    const STORAGE_UNIFORM: u32 = 2;

    enum Type {
        /// `true` for an image a shader writes to or reads without a sampler.
        Image(bool),
        Sampler,
        Struct,
        Array(u32, u32),
        Pointer(u32),
    }

    if words.len() < 5 || words[0] != 0x0723_0203 {
        return Err("glslang did not return SPIR-V".into());
    }
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut types: HashMap<u32, Type> = HashMap::new();
    let mut constants: HashMap<u32, u32> = HashMap::new();
    let mut slots: HashMap<u32, u32> = HashMap::new();
    let mut sets: HashMap<u32, u32> = HashMap::new();
    let mut blocks: Vec<u32> = Vec::new();
    let mut variables: Vec<(u32, u32, u32)> = Vec::new();
    let mut used: HashSet<u32> = HashSet::new();

    let mut at = 5;
    while at < words.len() {
        let length = (words[at] >> 16) as usize;
        let op = words[at] & 0xffff;
        if length == 0 || at + length > words.len() {
            return Err("the SPIR-V is cut short".into());
        }
        let args = &words[at + 1..at + length];
        match (op, args) {
            (OP_NAME, [id, text @ ..]) => {
                let bytes: Vec<u8> = text
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .take_while(|byte| *byte != 0)
                    .collect();
                names.insert(*id, String::from_utf8(bytes)?);
            }
            (OP_TYPE_IMAGE, [id, _, _, _, _, _, sampled, ..]) => {
                types.insert(*id, Type::Image(*sampled == 2));
            }
            (OP_TYPE_SAMPLER, [id]) => {
                types.insert(*id, Type::Sampler);
            }
            (OP_TYPE_STRUCT, [id, ..]) => {
                types.insert(*id, Type::Struct);
            }
            (OP_TYPE_ARRAY, [id, element, length]) => {
                types.insert(*id, Type::Array(*element, *length));
            }
            (OP_TYPE_POINTER, [id, _, pointee]) => {
                types.insert(*id, Type::Pointer(*pointee));
            }
            (OP_CONSTANT, [_, id, value]) => {
                constants.insert(*id, *value);
            }
            (OP_VARIABLE, [pointer, id, storage, ..]) => variables.push((*id, *pointer, *storage)),
            (OP_DECORATE, [id, DECORATION_BINDING, slot]) => {
                slots.insert(*id, *slot);
            }
            (OP_DECORATE, [id, DECORATION_SET, set]) => {
                sets.insert(*id, *set);
            }
            (OP_DECORATE, [id, DECORATION_BLOCK]) => blocks.push(*id),
            // The instructions that can take a variable: result type and
            // result come first, then the variable.
            (
                OP_LOAD
                | OP_ACCESS_CHAIN
                | OP_IN_BOUNDS_ACCESS_CHAIN
                | OP_IMAGE_TEXEL_POINTER
                | OP_COPY_OBJECT,
                [_, _, id, ..],
            ) => {
                used.insert(*id);
            }
            (OP_STORE, [id, ..]) => {
                used.insert(*id);
            }
            (OP_FUNCTION_CALL, [_, _, _, arguments @ ..]) => used.extend(arguments),
            _ => {}
        }
        at += length;
    }

    let mut bindings = Vec::new();
    for (id, pointer, storage) in variables {
        let resource = storage == STORAGE_UNIFORM_CONSTANT || storage == STORAGE_UNIFORM;
        if !resource || !used.contains(&id) {
            continue;
        }
        let name = names.get(&id).cloned().unwrap_or_default();
        let (Some(slot), Some(Type::Pointer(pointee))) = (slots.get(&id), types.get(&pointer))
        else {
            return Err(format!("`{name}` has no binding").into());
        };
        let mut pointee = *pointee;
        let mut count = 1;
        while let Some(Type::Array(element, length)) = types.get(&pointee) {
            count *= constants.get(length).copied().unwrap_or(1);
            pointee = *element;
        }
        let kind = match types.get(&pointee) {
            Some(Type::Image(false)) => Kind::Texture,
            Some(Type::Image(true)) => Kind::WritableTexture,
            Some(Type::Sampler) => Kind::Sampler,
            Some(Type::Struct) if storage == STORAGE_UNIFORM && blocks.contains(&pointee) => {
                Kind::Constants
            }
            _ => return Err(format!("`{name}` is of a kind this build does not know").into()),
        };
        if name.is_empty() {
            return Err(format!("binding {slot} has no name").into());
        }
        bindings.push(Binding {
            kind,
            name,
            slot: *slot,
            count,
            set: sets.get(&id).copied().unwrap_or(0),
        });
    }
    bindings.sort_by_key(|binding| (binding.set, binding.slot));
    Ok(bindings)
}
