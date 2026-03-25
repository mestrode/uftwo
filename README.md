# `uf2-file`

[![Crate](https://img.shields.io/crates/v/uf2-file.svg)](https://crates.io/crates/uf2-file)
[![Docs](https://docs.rs/uf2-file/badge.svg)](https://docs.rs/uf2-file)

A Rust library for working with the [UF2 file format](https://github.com/microsoft/uf2). This crate provides utilities for reading, writing, and validating UF2 files, which are commonly used for flashing firmware to microcontrollers.

## Features

- **Block Operations**: Read, write, and validate UF2 blocks with support for checksums and extensions.
- **File Handling**: High-level APIs for working with UF2 files, including validation and conversion.
- **Error Handling**: Comprehensive error types for block and file operations.
- **defmt Support**: Enable `defmt-03` feature to use [defmt](https://github.com/knurling-rs/defmt) `Format` on relevant types for embedded logging.

## Usage

### Basic Block Operations

```rust
use uf2_file::Block;

// Create a new UF2 block
let block = Block::new(0, &[0x01, 0x02, 0x03], None).unwrap();

// Validate the block
assert!(block.is_valid().unwrap());
```

### File Operations

```rust
use uf2_file::Uf2File;
use std::path::Path;

// Read a UF2 file
let uf2_file = Uf2File::read(Path::new("firmware.uf2")).unwrap();

// Validate the file
assert!(uf2_file.is_valid().unwrap());
```

## Features

- `file`: Enable file reading and writing functionality (enabled by default).
- `defmt-03`: Enable defmt logging support for embedded systems.
- `cli`: Enable command-line interface tools for UF2 file manipulation.

## See Also

This crate was forked and built on top of the [uftwo](https://crates.io/crates/uftwo) crate.

- uftwo [repo](https://crates.io/crates/uftwo), [crates.io](https://crates.io/crates/uftwo),  [docs](https://docs.rs/uftwo)

## License

This project is licensed under the [MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/) license.