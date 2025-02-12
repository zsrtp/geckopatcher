fn main() {
    if std::env::var("CARGO_FEATURE_PARALLEL").is_ok_and(|b| !b.is_empty()) {
        // println!("cargo::rustc-flags=-C target-feature=+atomics,+bulk-memory");
        println!("cargo::rustc-cfg=web_sys_unstable_apis");
        println!("cargo::warning=Building for multithreaded environment");
    } else {
        println!("cargo::warning=Building for singlethreaded environment");
    }
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CARGO_FEATURE_PARALLEL");
}
