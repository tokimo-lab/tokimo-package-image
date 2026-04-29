//! Build script: locate libvips from `bin/libvips/current` provided by deps.toml.
//!
//! - Linux/macOS: emit `cargo:rustc-link-search=native=...` and rpath so the
//!   linker finds `libvips` + `libgobject-2.0` declared by `#[link(...)]` in
//!   src/vips.rs, and so the binary loads them at runtime without needing
//!   LD_LIBRARY_PATH / DYLD_LIBRARY_PATH.
//! - Windows (msvc/mingw): emit search dir for any import lib (.dll.a / .lib);
//!   actual DLL loading happens at runtime via `LoadLibraryW` in src/vips.rs.
//!
//! Lookup order:
//!   1. `TOKIMO_DEP_LIBVIPS_DIR` env override (must point at the install dir
//!      containing `lib/`, `include/`, `bin/`).
//!   2. Walk up from `CARGO_MANIFEST_DIR` looking for `bin/libvips/current`.
#![allow(clippy::panic, clippy::expect_used, clippy::manual_assert)]

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=TOKIMO_DEP_LIBVIPS_DIR");

    let dep_dir = locate_libvips();
    let lib_dir = dep_dir.join("lib");
    let include_dir = dep_dir.join("include");

    if !lib_dir.is_dir() {
        panic!(
            "libvips install missing lib/: {} (run `pnpm deps --dep libvips`)",
            lib_dir.display()
        );
    }

    let lib_dir_canonical = lib_dir.canonicalize().unwrap_or(lib_dir.clone());
    println!("cargo:rustc-link-search=native={}", lib_dir_canonical.display());

    // GNU-ld rpath (`-Wl,-rpath,...`) is only valid for non-MSVC linkers.
    // MSVC uses /LIBPATH (already covered above) and resolves DLLs via PATH.
    let target_env_msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !target_env_msvc {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir_canonical.display());
    }

    if include_dir.is_dir() {
        let inc = include_dir.canonicalize().unwrap_or(include_dir);
        println!("cargo:include={}", inc.display());
    }

    // Re-run if the install dir was rebuilt by `pnpm deps`.
    println!("cargo:rerun-if-changed={}", lib_dir_canonical.display());
}

fn locate_libvips() -> PathBuf {
    if let Ok(p) = env::var("TOKIMO_DEP_LIBVIPS_DIR") {
        let pb = PathBuf::from(&p);
        if pb.is_dir() {
            return pb;
        }
        panic!("TOKIMO_DEP_LIBVIPS_DIR set but not a directory: {p}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut dir = manifest_dir.as_path();
    loop {
        let candidate = dir.join("bin").join("libvips").join("current");
        if candidate.is_dir() {
            return candidate;
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }

    panic!(
        "Could not find bin/libvips/current. Run `pnpm deps --dep libvips` from the workspace root, \
         or set TOKIMO_DEP_LIBVIPS_DIR."
    );
}
