use std::path::Path;

fn main() {
    // This crate lives in <repo>/generator; resolve paths from the repo root.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    // The header is the cbindgen output committed in the Lore repository
    // (https://github.com/EpicGames/lore), pinned via the `lore` submodule.
    let header = root.join("lore/lore-capi/lore.h");
    assert!(
        header.exists(),
        "{} not found; initialize the submodule with `git submodule update --init`",
        header.display()
    );

    // Keep the generated code compatible with the crate's declared
    // `rust-version` (e.g. no `offset_of!`, stabilized in 1.77).
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let msrv = manifest
        .lines()
        .find_map(|line| line.strip_prefix("rust-version"))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("no rust-version in Cargo.toml");

    let bindings = bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .rust_target(
            msrv.parse()
                .unwrap_or_else(|e| panic!("unsupported rust-version {msrv:?}: {e}")),
        )
        // Everything the C API exports carries a lore_/LORE_ prefix (see
        // lore/cbindgen.toml upstream); this keeps libc/stdarg noise out.
        .allowlist_item("(lore|LORE)_.*")
        .prepend_enum_name(false)
        // Generate a `Lore` struct that loads all symbols from the dynamic
        // library at runtime via libloading, instead of link-time bindings.
        .dynamic_library_name("Lore")
        .dynamic_link_require_all(true)
        .generate()
        .expect("failed to generate bindings for lore.h");

    let out = root.join("src/bindings.rs");
    bindings
        .write_to_file(&out)
        .expect("failed to write src/bindings.rs");
    println!("wrote {}", out.display());

    // Build the lore dynamic library from the same submodule revision the
    // bindings were generated from, and copy it into lib/ to be checked in.
    // Only a release build for the moment; we can add debug if it becomes
    // necessary.
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "-p", "lore"])
        .current_dir(root.join("lore"))
        .status()
        .expect("failed to run cargo build in the lore submodule");
    assert!(status.success(), "building the lore dynamic library failed");

    let lib_dir = root.join("lib");
    std::fs::create_dir_all(&lib_dir).expect("failed to create lib/");
    let dll = format!(
        "{}lore{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    // The .pdb only exists when the profile emits debug info; copy it when
    // present so debuggers get symbols matching the .dll.
    for file in [dll.as_str(), "lore.pdb"] {
        let src = root.join("lore/target/release").join(file);
        if src.exists() {
            let dest = lib_dir.join(file);
            std::fs::copy(&src, &dest)
                .unwrap_or_else(|e| panic!("failed to copy {} to lib/: {e}", src.display()));
            println!("wrote {}", dest.display());
        }
    }
}
