# 📖 lore-sys

[![Actions Status](https://github.com/Traverse-Research/Lore-rust-bindings/actions/workflows/ci.yml/badge.svg)](https://github.com/Traverse-Research/Lore-rust-bindings/actions)
[![Latest version](https://img.shields.io/crates/v/lore-sys.svg?logo=rust)](https://crates.io/crates/lore-sys)
[![Documentation](https://docs.rs/lore-sys/badge.svg)](https://docs.rs/lore-sys)
[![MSRV](https://img.shields.io/badge/rustc-1.74.0+-ab6000.svg)](https://blog.rust-lang.org/2023/11/16/Rust-1.74.0.html)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
[![Contributor Covenant](https://img.shields.io/badge/contributor%20covenant-v1.4%20adopted-ff69b4.svg)](./CODE_OF_CONDUCT.md)

[![Banner](banner.png)](https://traverseresearch.nl)

Raw Rust bindings for the C API of [Lore], Epic Games' open source version
control system.

The bindings ([`src/bindings.rs`](src/bindings.rs)) are pregenerated with
[bindgen] from the `lore-capi/lore.h` header in the [`lore`](lore) git
submodule, which is itself the cbindgen output committed in the Lore
repository. Building this crate therefore needs no build script, bindgen,
libclang or initialized submodule. The Lore dynamic library (`lore.dll` /
`liblore.so` / `liblore.dylib`, built from Lore's `lore` crate as a `cdylib`)
is not linked at build time; it is loaded at runtime through [libloading] via
the generated [`Lore`] struct.

[Lore]: https://github.com/EpicGames/lore
[bindgen]: https://crates.io/crates/bindgen
[libloading]: https://crates.io/crates/libloading

## Usage

Add this to your Cargo.toml:

```toml
[dependencies]
lore-sys = "0.0.0"
```

Load the library and call into the C API:

```rust,no_run
use lore_sys::Lore;

let lore = unsafe { Lore::load("path/to/lore.dll") }.expect("failed to load Lore");
// Or resolve `lore.dll`/`liblore.so`/`liblore.dylib` from the system search path:
// let lore = unsafe { Lore::load(lore_sys::library_filename()) }.expect("failed to load Lore");

unsafe { lore.lore_shutdown() };
```

## Updating the bindings

Bump the [`lore`](lore) submodule to the desired revision and regenerate
[`src/bindings.rs`](src/bindings.rs) from its header with the
[`generator`](generator) tool. Running it requires `libclang` (see the [bindgen
requirements](https://rust-lang.github.io/rust-bindgen/requirements.html)):

```sh
git submodule update --init
cargo run --manifest-path generator/Cargo.toml
```

The submodule is marked `shallow` in [`.gitmodules`](.gitmodules), so
initializing it only fetches the pinned revision rather than the full Lore
history. It is currently pinned to `348e9407f29f59626dec2669c232897cef1cb33e`
(interface version `0.8.4-nightly`).
