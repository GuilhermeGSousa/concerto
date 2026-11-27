#![allow(missing_docs, reason = "build script of the raw Jolt bindings")]

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // Include root for the canonical `#include <Jolt/...>` form.
    let jolt_root = manifest.join("vendor/JoltPhysics");

    let jolt_sources = jolt_root.join("Jolt");
    assert!(
        jolt_sources.is_dir(),
        "Jolt sources not found at {}; the JoltPhysics git submodule is not \
         initialised — run `git submodule update --init`",
        jolt_sources.display()
    );

    let mut sources = Vec::new();
    collect_cpp(&jolt_sources, &mut sources);
    collect_cpp(&manifest.join("csrc"), &mut sources);
    // Deterministic archive contents regardless of directory-walk order.
    sources.sort();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(&jolt_root)
        .include(manifest.join("csrc"))
        // Every translation unit (Jolt + shim) must be compiled with the same
        // JPH_* configuration: Jolt packs the config into JPH_VERSION_ID and
        // aborts at RegisterTypes() if the caller and library disagree. Keep
        // this define list as the single source of truth.
        //
        // NDEBUG keeps Jolt's asserts out of every profile, and object layers are 32
        // bits wide, to hold a body's collision layers and their filter (part
        // of the version check). All other feature bits are deliberately off.
        .define("NDEBUG", None)
        .define("JPH_OBJECT_LAYER_BITS", "32")
        // Jolt's own build disables FMA contraction on GCC: contracted
        // multiply-adds change rounding in ways that break collision-detection
        // precision assumptions.
        .flag_if_supported("-ffp-contract=off")
        // Documented GCC false positive in Jolt.
        .flag_if_supported("-Wno-stringop-overflow")
        // Vendored third-party code; its warnings are not actionable here.
        .warnings(false)
        .files(&sources);
    build.compile("jolt");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-changed=vendor/JoltPhysics/Jolt");
}

fn collect_cpp(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("failed to read Jolt source directory") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_cpp(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "cpp") {
            out.push(path);
        }
    }
}
