<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>aot-lang</b>
    <br>
    <sub><sup>AOT COMPILER</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/aot-lang"><img alt="Crates.io" src="https://img.shields.io/crates/v/aot-lang"></a>
    <a href="https://crates.io/crates/aot-lang"><img alt="Downloads" src="https://img.shields.io/crates/d/aot-lang?color=%230099ff"></a>
    <a href="https://docs.rs/aot-lang"><img alt="docs.rs" src="https://img.shields.io/docsrs/aot-lang"></a>
    <a href="https://github.com/jamesgober/aot-lang/actions"><img alt="CI" src="https://github.com/jamesgober/aot-lang/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        <strong>aot-lang</strong> is the ahead-of-time compiler of the <code>-lang</code> language-construction family. It takes functions in the <a href="https://docs.rs/ir-lang"><code>ir-lang</code></a> intermediate representation, lowers each to object code, and lays them out together into a single linked <strong>image</strong>&mdash;a self-contained artifact with its code, a symbol table, and an entry point, ready to be inspected, serialized, or loaded.
    </p>
    <p>
        It is the batch counterpart to a just-in-time compiler: where a JIT runs a function the moment it is generated, aot-lang compiles the whole program up front. It is the end of the pipeline a front-end follows&mdash;parse, type-check, lower to IR, then compile ahead of time. The surface is deliberately small, and the object-code encoding is little-endian on every target, so an image built on one host is byte-identical on another.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.85+</strong> (Rust 2024 edition). <code>#![no_std]</code>-capable (needs only <code>alloc</code>); <code>#![forbid(unsafe_code)]</code>.
    </p>
    <blockquote>
        <strong>Status: pre-1.0 (<code>v0.2.0</code>), in active development.</strong> The public API is being designed across the 0.x series and frozen at <code>1.0.0</code>. See <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a> and <a href="./dev/ROADMAP.md"><code>ROADMAP</code></a>.
    </blockquote>
</div>

<hr>
<br>

## Installation

```toml
[dependencies]
aot-lang = "0.2"
ir-lang = "1"
```

You build the input functions with `ir-lang`, so it belongs alongside `aot-lang`. For a `no_std` build, disable default features: `aot-lang = { version = "0.2", default-features = false }`.

<br>
<hr>
<br>

## How it works

A compile is two stages, one per dependency it wires:

1. **Lower** — each `ir_lang::Function` is validated and compiled to a [`codegen-lang`](https://docs.rs/codegen-lang) `Program` of register bytecode. Only well-formed SSA is ever lowered.
2. **Encode and link** — each program is encoded into a `.text` section with a symbol at its start, and [`linker-lang`](https://docs.rs/linker-lang) places the objects at addresses, resolves the entry point, and produces the image.

A malformed function is rejected in the first stage as `AotError::Codegen`; a name collision or a missing entry point fails the second as `AotError::Link`.

<br>
<hr>
<br>

## Quick Start

Compile a single function into an image, with the function as the entry point:

```rust
use aot_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn square(x: int) -> int { x * x }
let mut b = Builder::new("square", &[Type::Int], Type::Int);
let x = b.block_params(b.entry())[0];
let sq = b.bin(BinOp::Mul, x, x);
b.ret(Some(sq));

let image = compile(&b.finish()).expect("square is well-formed");

assert_eq!(image.entry(), Some(0));
assert_eq!(image.symbol("square"), Some(0));
assert!(!image.section(".text").unwrap().data().is_empty());
```

Place several functions in one image with the [`Compiler`](./docs/API.md#compiler) builder, choosing the base address and entry point:

```rust
use aot_lang::Compiler;
use ir_lang::{Builder, BinOp, Type};

// fn add(a: int, b: int) -> int { a + b }
let mut add = Builder::new("add", &[Type::Int, Type::Int], Type::Int);
let a = add.block_params(add.entry())[0];
let b = add.block_params(add.entry())[1];
let sum = add.bin(BinOp::Add, a, b);
add.ret(Some(sum));

// fn answer() -> int { 42 }
let mut answer = Builder::new("answer", &[], Type::Int);
let c = answer.iconst(42);
answer.ret(Some(c));

let mut compiler = Compiler::new();
compiler.base_address(0x40_0000).entry("add");
compiler.add(&add.finish()).unwrap();
compiler.add(&answer.finish()).unwrap();
let image = compiler.link().unwrap();

assert_eq!(image.entry(), Some(0x40_0000));
assert_eq!(image.symbol("add"), Some(0x40_0000));
assert!(image.symbol("answer").is_some());
```

<br>
<hr>

## API Overview

For a complete reference with examples for every item, see [`docs/API.md`](./docs/API.md).

- [`compile`](./docs/API.md#compile) — ahead-of-time compile one function into an image, with it as the entry point
- [`Compiler`](./docs/API.md#compiler) — a builder that accumulates functions and links them into one image, with a configurable base address and entry point
- [`AotError`](./docs/API.md#aoterror) — the failure type, distinguishing the code-generation stage from the link stage
- [`Image`](./docs/API.md#image) — the linked result: laid-out code, a resolved symbol table, and an entry point (re-exported from `linker-lang`)
- [`OutputSection`](./docs/API.md#outputsection) — one laid-out section of an image (re-exported from `linker-lang`)

<br>

### Highlights

- **Small surface** — one function, one builder, one error type. Nothing to configure that the task does not need.
- **Two-stage pipeline** — code generation and linking are separate, wired crates, and a failure names which stage it came from.
- **Deterministic output** — the object-code encoding is fixed little-endian, so recompiling the same IR yields byte-identical images across hosts.
- **`no_std`-ready** — needs only `alloc`, performs no I/O, and forbids `unsafe`. The whole pipeline drops the standard library together.
- **Cross-platform** — Linux, macOS, and Windows on x86-64 and ARM64, verified in CI on all three operating systems and at the MSRV.

<br>
<hr>
<br>

## Contributing

See [`REPS.md`](./REPS.md) for the engineering standards and the definition of done, and [`dev/ROADMAP.md`](./dev/ROADMAP.md) for the plan to 1.0. Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b> &mdash; <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober <me@jamesgober.com>.</strong></sup>
</div>
