use resvg;

fn main() {
    // println!("cargo::rerun-if-changed=assets/manifest.json.in");

    // Replace the {{}} in the manifest.json file with the actual values
    let manifest = include_str!("assets/manifest.json.in");
    let short_name = if cfg!(feature = "generic_patch") { "gecko-patcher-pwa" } else { "tpgz-patcher-pwa" };
    let name = if cfg!(feature = "generic_patch") { "Gecko Patcher" } else { "TPGZ Patcher" };
    let manifest = manifest.replace("{{name}}", name);
    let manifest = manifest.replace("{{short_name}}", short_name);

    std::fs::write("assets/manifest.json", manifest).unwrap();

    // Generate the icons
    let tree = {
        let mut options = resvg::usvg::Options::default();
        options.resources_dir = Some("assets".into());
        options.fontdb_mut().load_system_fonts();
        let svg_data = std::fs::read(if cfg!(feature = "generic_patch") {"assets/Gecko2.svg"} else {"assets/Gecko.svg"}).unwrap();
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
    pixmap_192.save_png("assets/icon_ios_touch_192.png").unwrap();
    pixmap_256.save_png("assets/icon-256.png").unwrap();
    pixmap_256.save_png("assets/favicon.ico").unwrap();
    pixmap_512.save_png("assets/maskable_icon_x512.png").unwrap();
    pixmap_1024.save_png("assets/icon-1024.png").unwrap();
}
