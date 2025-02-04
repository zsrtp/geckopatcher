use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let project_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let cargo_dir = project_dir.join(".cargo");
    let assets = project_dir.join("assets");
    if env::var_os("CARGO_FEATURE_PARALLEL").is_some_and(|v| v.is_empty()) {
        if fs::exists(&cargo_dir).is_ok_and(|b| !b) {
            fs::create_dir(&cargo_dir).unwrap();
        }
        fs::copy(assets.join("cargo_config_singlethreaded.toml"), cargo_dir.join("config.toml")).unwrap();
    } else {
        if fs::exists(&cargo_dir).is_ok_and(|b| !b) {
            fs::create_dir(&cargo_dir).unwrap();
        }
        fs::copy(assets.join("cargo_config_multithreaded.toml"), cargo_dir.join("config.toml")).unwrap();
    }
    println!("cargo::rerun-if-changed=build.rs");
}