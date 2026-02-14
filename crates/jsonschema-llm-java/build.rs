use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    
    // Generate C bindings
    let bindings = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(cbindgen::Config::from_file("cbindgen.toml").unwrap())
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(out_dir).join("jsonschema_llm.h");
    bindings
        .write_to_file(&out_path);
        
    // Copy to target/include for Java binding
    let target_dir = PathBuf::from("../../target/include");
    fs::create_dir_all(&target_dir).unwrap();
    fs::copy(&out_path, target_dir.join("jsonschema_llm.h")).unwrap();
}
