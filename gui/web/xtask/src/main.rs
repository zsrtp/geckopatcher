use resvg;
use toml;
use std::path::PathBuf;

use serde_derive::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    pub name: String,
    pub short_name: String,
    pub icon_path: PathBuf,
    pub multithreaded: bool,
}

fn main() {
    // Replace the {{}} in the manifest.json file with the actual values
    let manifest = include_str!("../../assets/manifest.json.in");
    let config_str = include_str!("../../assets/config.toml");
    let config: Config = toml::from_str::<Config>(config_str).unwrap();
    let manifest = manifest.replace("{{name}}", &config.name);
    let manifest = manifest.replace("{{short_name}}", &config.short_name);

    let dist = PathBuf::from(std::env::var_os("TRUNK_STAGING_DIR").expect("unable eval dist dir"));
    let src = PathBuf::from(std::env::var_os("TRUNK_SOURCE_DIR").expect("unable eval src dir"));

    std::fs::write(dist.join("manifest.json"), manifest).unwrap();
    if config.multithreaded {
        std::fs::copy(src.join("assets/cargo_config_multithreaded.toml"), src.join(".cargo/config.toml")).unwrap();
    } else {
        std::fs::copy(src.join("assets/cargo_config_singlethreaded.toml"), src.join(".cargo/config.toml")).unwrap();
    }

    // Generate the icons
    let tree = {
        let mut options = resvg::usvg::Options::default();
        options.resources_dir = None;
        options.fontdb_mut().load_system_fonts();
        let svg_data = std::fs::read(&config.icon_path).unwrap();
        resvg::usvg::Tree::from_data(&svg_data, &options).unwrap()
    };
    let pixmap_size = tree.size().to_int_size();
    let mut pixmap_192 = resvg::tiny_skia::Pixmap::new(192, 192).unwrap();
    let mut pixmap_256 = resvg::tiny_skia::Pixmap::new(256, 256).unwrap();
    let mut pixmap_512 = resvg::tiny_skia::Pixmap::new(512, 512).unwrap();
    let mut pixmap_1024 = resvg::tiny_skia::Pixmap::new(1024, 1024).unwrap();
    let transform_192 = resvg::tiny_skia::Transform::default()
        .post_scale(192.0 / (pixmap_size.width() as f32), 192.0 / (pixmap_size.height() as f32));
    let transform_256 = resvg::tiny_skia::Transform::default()
        .post_scale(256.0 / (pixmap_size.width() as f32), 256.0 / (pixmap_size.height() as f32));
    let transform_512 = resvg::tiny_skia::Transform::default()
        .post_scale(512.0 / (pixmap_size.width() as f32), 512.0 / (pixmap_size.height() as f32));
    let transform_1024 = resvg::tiny_skia::Transform::default()
        .post_scale(1024.0 / (pixmap_size.width() as f32), 1024.0 / (pixmap_size.height() as f32));
    resvg::render(&tree, transform_192, &mut pixmap_192.as_mut());
    resvg::render(&tree, transform_256, &mut pixmap_256.as_mut());
    resvg::render(&tree, transform_512, &mut pixmap_512.as_mut());
    resvg::render(&tree, transform_1024, &mut pixmap_1024.as_mut());
    pixmap_192.save_png(dist.join("icon_ios_touch_192.png")).unwrap();
    pixmap_256.save_png(dist.join("icon-256.png")).unwrap();
    pixmap_256.save_png(dist.join("favicon.ico")).unwrap();
    pixmap_512.save_png(dist.join("maskable_icon_x512.png")).unwrap();
    pixmap_1024.save_png(dist.join("icon-1024.png")).unwrap();
}
