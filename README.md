# lore-rs

[![Actions Status](https://github.com/Traverse-Research/lore-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Traverse-Research/lore-rs/actions)
[![MSRV](https://img.shields.io/badge/rustc-1.74.0+-ab6000.svg)](https://blog.rust-lang.org/2023/11/16/Rust-1.74.0.html)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
[![Contributor Covenant](https://img.shields.io/badge/contributor%20covenant-v1.4%20adopted-ff69b4.svg)](./CODE_OF_CONDUCT.md)

[![Banner](banner.png)](https://traverseresearch.nl)

# About

Rust bindings and some helper functions for the C API of [Lore], Epic Games' open source version
control system.

Lore itself is built in Rust, but linking to it dynamically goes through its C API. The `lore-sys` crate contains the raw bindings; the `lore-rs` crate adds some ergonomic helpers on top.

The raw bindings have been automatically generated using [bindgen](https://github.com/rust-lang/rust-bindgen).

## Usage

The crates are not published on crates.io; depend on them via git:

```toml
[dependencies]
lore-rs = { git = "https://github.com/Traverse-Research/lore-rs" }

[build-dependencies]
lore-bin = { git = "https://github.com/Traverse-Research/lore-rs" }
```

The bindings load the Lore dynamic library (`lore.dll` / `liblore.so` /
`liblore.dylib`) at runtime, so you need a copy of that library before
anything can be called. There are two ways to get one.

**A - download it in your build script with `lore_bin::fetch_binary`**

In your `build.rs`, fetch the library for the target being compiled:

```rust,no_run
// build.rs
let target = lore_bin::Target::from_build_env();
let library_path = lore_bin::fetch_binary(target);
```

This downloads the library from the official [Lore releases] into `OUT_DIR`
and returns its path. Getting it from there to a place your executable can
load it from is up to you; `lore_bin::library_file_name(target)` gives the
file name the OS loader expects. Then load it at runtime:

```rust,no_run
let lore = unsafe { lore_rs::load("path/to/lore.dll") }?;
```

**B - provide your own copy**

If you want to opt out of the downloading step, grab the archive for your
target from the [Lore releases] yourself, extract it, and load the library
from wherever you put it:

```rust,no_run
let lore = unsafe { lore_rs::load("your/path/to/lore.dll") }?;
```

Either way, the library must match the version these bindings were generated
for (`LORE_VERSION` in [`lore-bin`](lore-bin/src/lib.rs)); a mismatch is
undefined behaviour.

`load` may be called any number of times and hands back the same library.
Lore is process-global: it is never unloaded, and `Lore::shutdown` is a
deliberate, final act rather than something a value's drop could do.

Three layers sit on top of each other: the raw `lore_sys` bindings, one safe
function per command in `lore_rs::call`, and handle types that return data.
Reading one file out of a repository, and storing content back, looks like
this:

```rust,no_run
use lore_rs::{GlobalArgs, PutItem, Repository, Store, StoreLocation, StoreOptions};

let lore = unsafe { lore_rs::load("path/to/lore.dll") }?;

// What the server knows about the repository; needs no local checkout.
let info = lore.repository_info(&GlobalArgs::default(), "lore://host:41337/project")?;

// A local repository instance: a directory holding `.lore`. Repository,
// branch and revision commands run against it. `main@LATEST` resolves the
// way Lore's own CLI does: the local tip unless the server is strictly ahead.
let repository = Repository::new(
    lore,
    GlobalArgs {
        repository_path: "path/to/checkout".into(),
        ..Default::default()
    },
);
let tip = repository
    .revision(&format!("{}@LATEST", info.default_branch_name))?
    .expect("the branch points at a revision")
    .revision;

// Lore's storage API: a store is opened by location and serves any
// repository, so every read names the one it is about.
let store = Store::open(
    lore,
    &GlobalArgs::default(),
    StoreOptions {
        location: StoreLocation::OnDisk("path/to/checkout".into()),
        remote_url: Some("lore://host:41337".into()),
        local_cache: true,
        ..Default::default()
    },
)?;
let tree = store.load_revision_tree(info.id, tip)?;
let node = tree.node_at("models/a.gltf")?;
let bytes = store.get(tree.repository(), node.address)?;

// Writing goes through the same handle. Nothing in the repository points at
// the address afterwards, so it is the caller's to keep.
let address = store.put(PutItem {
    repository: info.id,
    data: b"cooked output",
    ..Default::default()
})?;
```

Everything the handle types do not cover is reachable through `lore_rs::call`,
and everything else through the raw `lore_sys` functions on `Lore`. For the
usage of Lore itself, see the [Lore documentation](https://github.com/EpicGames/lore).

[Lore]: https://github.com/EpicGames/lore
[Lore releases]: https://github.com/EpicGames/lore/releases


## Updating the bindings

Updating to a new Lore version is three steps, all of which CI verifies stay
in sync:

1. Bump the [`lore`](lore) submodule to the desired revision. This must be a
   tagged release — prebuilt binaries only exist for releases:

   ```sh
   git submodule update --init
   git -C lore fetch --depth 1 origin tag v0.8.6
   git -C lore checkout v0.8.6
   ```

2. Update the hardcoded `LORE_VERSION` in
   [`lore-bin/src/lib.rs`](lore-bin/src/lib.rs) to the same version, so
   `fetch_binary` downloads binaries matching the submodule.

3. Regenerate the bindings from the submodule's headers:

   ```sh
   cargo run --manifest-path generator/Cargo.toml
   ```
   
This should be done on Linux as the bindings differ slightly per platform.


Commit the submodule bump, the version constant and the regenerated
`lore-sys/src/bindings.rs` together. CI fails if `LORE_VERSION` doesn't match
the tag the submodule is pinned to, or if the committed bindings differ from
freshly generated ones.

The submodule is marked `shallow` in [`.gitmodules`](.gitmodules), so
initializing it only fetches the pinned revision rather than the full Lore
history.
