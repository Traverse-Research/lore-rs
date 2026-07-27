use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    let header = root.join("lore/lore-capi/lore.h");
    assert!(
        header.exists(),
        "{} not found; initialize the submodule with `git submodule update --init`",
        header.display()
    );

    let manifest = std::fs::read_to_string(root.join("lore-sys/Cargo.toml")).unwrap();
    let msrv = manifest
        .lines()
        .find_map(|line| line.strip_prefix("rust-version"))
        .and_then(|rest| rest.split('"').nth(1))
        .expect("no rust-version in Cargo.toml");

    let bindings = bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .clang_arg("-fparse-all-comments")
        .formatter(bindgen::Formatter::Prettyplease)
        .rust_target(
            msrv.parse()
                .unwrap_or_else(|e| panic!("unsupported rust-version {msrv:?}: {e}")),
        )
        .allowlist_item("(lore|LORE)_.*")
        .prepend_enum_name(false)
        .dynamic_library_name("Lore")
        .dynamic_link_require_all(true)
        .generate()
        .expect("failed to generate bindings for lore.h");

    let out = root.join("lore-sys/src/bindings.rs");
    bindings
        .write_to_file(&out)
        .expect("failed to write lore-sys/src/bindings.rs");
    println!("wrote {}", out.display());
}
