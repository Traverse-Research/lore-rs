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

    let bindings = bindgen::Builder::default()
        .header(header.to_str().unwrap())
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
}
