use std::{env, fs, path::PathBuf};

use naga::{ShaderStage, back::spv, front::glsl, valid};

fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));

    for (file, stage) in [
        ("triangle.vert", ShaderStage::Vertex),
        ("triangle.frag", ShaderStage::Fragment),
    ] {
        let path = PathBuf::from("shaders").join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        let source = fs::read_to_string(&path).expect("Failed to read shader source");
        let module = glsl::Frontend::default()
            .parse(&glsl::Options::from(stage), &source)
            .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
        let info = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let options = spv::Options {
            lang_version: (1, 3),
            ..Default::default()
        };
        let pipeline = spv::PipelineOptions {
            shader_stage: stage,
            entry_point: "main".into(),
        };
        let words = spv::write_vec(&module, &info, &options, Some(&pipeline))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        fs::write(output.join(format!("{file}.spv")), bytes).expect("Failed to write SPIR-V");
    }
}
