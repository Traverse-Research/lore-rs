# lore-rs

[![Actions Status](https://github.com/Traverse-Research/Lore-rust-bindings/actions/workflows/ci.yml/badge.svg)](https://github.com/Traverse-Research/Lore-rust-bindings/actions)
[![Latest version](https://img.shields.io/crates/v/lore-sys.svg?logo=rust)](https://crates.io/crates/lore-sys)
[![Documentation](https://docs.rs/lore-sys/badge.svg)](https://docs.rs/lore-sys)
[![MSRV](https://img.shields.io/badge/rustc-1.74.0+-ab6000.svg)](https://blog.rust-lang.org/2023/11/16/Rust-1.74.0.html)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
[![Contributor Covenant](https://img.shields.io/badge/contributor%20covenant-v1.4%20adopted-ff69b4.svg)](./CODE_OF_CONDUCT.md)

[![Banner](banner.png)](https://traverseresearch.nl)

Rust bindings and some helper functions for the C API of [Lore], Epic Games' open source version
control system.

Lore itself is build in rust but for anyone wanting to link to it dynamically for whatever reason, you need to go through the C API. This crate contains the bindings and some helpers to make that more ergomic.

The raw bindings have been automatically generated using BindGen.


A seperate branch also contains pregenerated dynamic library artifacts for Windows, MacOS and Linux, all release builds.

# Depedencies

[Lore](https://github.com/EpicGames/lore)
[bindgen](https://crates.io/crates/bindgen)
[libloading](https://crates.io/crates/libloading)

## Usage

Loading the library:

```rust,no_run
use lore_rs::Lore;

let lore = unsafe { Lore::new("path/to/lore.dll") }.expect("failed to load Lore");
```
After that you can call into the bindings.

For the actual usage of Lore, you can see all the documentation regarding Lore [there](https://github.com/epicgames/lore)

## Updating the bindings
Bump the [`lore`](lore) submodule to the desired revision and run the generator.

```sh
git submodule update --init
cargo run --manifest-path generator/Cargo.toml

```

The submodule is marked `shallow` in [`.gitmodules`](.gitmodules), so
initializing it only fetches the pinned revision rather than the full Lore
history.
