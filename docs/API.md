<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>aot-lang</b><br>
    <sub><sup>API REFERENCE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>API</span>
        <span>&nbsp;│&nbsp;</span>
        <a href="../dev/ROADMAP.md" title="Roadmap"><b>ROADMAP</b></a>
    </sup>
</div>
<br>

> **Status: stable (1.0).** The surface below is the `1.0` contract: it follows [Semantic Versioning](#semver) and will not change in a breaking way before `2.0`. Both error types are `#[non_exhaustive]`, so new failure reasons stay additive. See [`dev/ROADMAP.md`](../dev/ROADMAP.md).

Ahead-of-time compile the [`ir-lang`](https://docs.rs/ir-lang) intermediate representation into a single linked [`Image`](#image): lower each function to object code with [`codegen-lang`](https://docs.rs/codegen-lang), encode it, and lay the objects out with [`linker-lang`](https://docs.rs/linker-lang).

<br>

## Table of Contents

- [Installation](#installation)
- [Pipeline model](#pipeline-model)
- [Public API](#public-api)
  - [`compile`](#compile)
  - [`Compiler`](#compiler)
  - [`AotError`](#aoterror)
  - [`Image`](#image)
  - [`OutputSection`](#outputsection)
- [Object-code format](#object-code-format)
- [Feature flags](#feature-flags)
- [SemVer](#semver)

<br>
<hr>
<br>

## Installation

```toml
[dependencies]
aot-lang = "1"
ir-lang = "1"
```

```bash
cargo add aot-lang ir-lang
```

You build the input functions with `ir-lang`, so it belongs in your dependencies alongside `aot-lang`. The crate is `#![no_std]`-capable: build with `default-features = false` to drop the standard library and depend only on `alloc`.

<br>
<hr>
<br>

## Pipeline model

A compile takes one or more [`ir_lang::Function`](https://docs.rs/ir-lang)s and produces a single [`Image`](#image). Two stages run, one per dependency the crate wires:

- **Lower.** Each function is compiled to a [`codegen_lang::Program`](https://docs.rs/codegen-lang) — a stream of register bytecode — after being checked with `Function::validate`, so only well-formed SSA is ever lowered. A function that fails validation is rejected here as [`AotError::Codegen`](#aoterror).
- **Encode and link.** Each program is encoded into the bytes of a `.text` section, with a symbol at its start named for the function, and [`linker-lang`](https://docs.rs/linker-lang) places the objects end to end from the base address, resolving the entry point. Two functions that share a name collide, and the link fails as [`AotError::Link`](#aoterror).

The object-code byte encoding is little-endian on every target, so an image compiled on one host is byte-identical on another. The result is an [`Image`](#image) whose `.text` bytes are final and whose symbol table records where each function landed. Each function's bytes are one self-describing record in the [object-code format](#object-code-format). aot-lang produces images; it ships no loader or interpreter.

<br>
<hr>
<br>

## Public API

### `compile`

```rust
pub fn compile(func: &ir_lang::Function) -> Result<Image, AotError>
```

Ahead-of-time compiles a single function into a linked [`Image`](#image), with the function itself as the entry point. This is the shortcut for the common case of one function; reach for [`Compiler`](#compiler) when you have several to place in one image, or need to set the base address or entry point.

The function is lowered at base address `0`, so its symbol and the image's entry both resolve to `0`.

**Parameters**

- `func` — the function to compile, in SSA form as produced by `ir_lang::Builder`. It is validated during lowering.

**Returns**

- `Ok(Image)` — the linked image: one `.text` section holding the function's object code, a symbol table with the function at its address, and the entry point set to it.
- `Err(AotError::Codegen(_))` — `func` did not pass `Function::validate`; the wrapped reason names the offending block or value. The link step cannot fail for a single well-formed function.

**Examples**

Compile a function and read back its entry point and code:

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

Compile a loop — a back-edge that carries values across blocks — and confirm it lays out as a single-entry image:

```rust
use aot_lang::compile;
use ir_lang::{Builder, BinOp, Type};

// fn sum_to(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }
let mut b = Builder::new("sum_to", &[Type::Int], Type::Int);
let n0 = b.block_params(b.entry())[0];
let header = b.create_block(&[Type::Int, Type::Int]);
let body = b.create_block(&[]);
let exit = b.create_block(&[]);

let zero = b.iconst(0);
b.jump(header, &[n0, zero]);

b.switch_to(header);
let n = b.block_params(header)[0];
let acc = b.block_params(header)[1];
let z = b.iconst(0);
let more = b.bin(BinOp::Gt, n, z);
b.branch(more, body, &[], exit, &[]);

b.switch_to(body);
let acc2 = b.bin(BinOp::Add, acc, n);
let one = b.iconst(1);
let n2 = b.bin(BinOp::Sub, n, one);
b.jump(header, &[n2, acc2]);

b.switch_to(exit);
b.ret(Some(acc));

let image = compile(&b.finish()).expect("sum_to is well-formed");
assert_eq!(image.entry(), Some(0));
assert_eq!(image.len(), 1); // one .text section
```

Reject a malformed function:

```rust
use aot_lang::{compile, AotError};
use ir_lang::{Builder, Type};

// Declares an int return but never returns a value.
let func = Builder::new("bad", &[], Type::Int).finish();
assert!(matches!(compile(&func), Err(AotError::Codegen(_))));
```

<br>

### `Compiler`

```rust
pub struct Compiler { /* private fields */ }
```

Accumulates functions and links them into one [`Image`](#image). Each function you [`add`](#compileradd) is compiled to object code and held; [`link`](#compilerlink) places them all in a single image. This is the multi-function counterpart to [`compile`](#compile): use it to build an image from several functions, to choose the base address the image is laid out at, or to name the entry point.

Functions are placed in the order they are added, each defining a symbol under its own name. Two functions with the same name collide — the link then fails with [`AotError::Link`](#aoterror). Implements `Default`, equivalent to [`Compiler::new`](#compilernew): base address `0`, no entry point.

**Methods**

| Method | Returns | Description |
|---|---|---|
| `new()` | `Compiler` | An empty compiler with the default configuration. Same as `Compiler::default()`. |
| `base_address(addr)` | `&mut Compiler` | Sets the address the image is laid out from. The first function is placed here; the rest follow. |
| `entry(symbol)` | `&mut Compiler` | Sets the symbol whose address becomes the image's entry point. It must name a function added before `link`. |
| `add(func)` | `Result<&mut Compiler, AotError>` | Compiles `func` to object code and appends it. Fails with `Codegen` if `func` does not validate. |
| `link()` | `Result<Image, AotError>` | Links every added function into one image. Fails with `Link` on a duplicate name or an undefined entry. |

The configuration methods return `&mut Self`, so bind the compiler first and then configure and feed it — this is what makes adding functions in a loop natural.

#### `Compiler::new`

```rust
pub fn new() -> Self
```

Creates an empty compiler: base address `0`, no entry point. Identical to `Compiler::default()`.

#### `Compiler::base_address`

```rust
pub fn base_address(&mut self, addr: u64) -> &mut Self
```

Sets the address the image is laid out from. Every resolved symbol address is relative to it.

- `addr` — the address the first function is placed at.

#### `Compiler::entry`

```rust
pub fn entry(&mut self, symbol: impl Into<String>) -> &mut Self
```

Sets the symbol whose address becomes the image's entry point.

- `symbol` — the name of a function that must be added before [`link`](#compilerlink), or the link fails with `LinkError::UndefinedEntry`.

#### `Compiler::add`

```rust
pub fn add(&mut self, func: &ir_lang::Function) -> Result<&mut Self, AotError>
```

Compiles `func` to object code and appends it to the image being built. The function is lowered immediately, so a malformed one is reported here rather than deferred to [`link`](#compilerlink).

- `func` — the function to compile, in SSA form. It is validated during lowering.

Returns `Err(AotError::Codegen(_))` if `func` does not pass `Function::validate`.

#### `Compiler::link`

```rust
pub fn link(&self) -> Result<Image, AotError>
```

Links every added function into one [`Image`](#image). The functions are placed end to end from the base address, each defining a symbol under its name, and the entry point — if one was set — resolves to its address.

Returns `Err(AotError::Link(_))` if two functions share a name (`LinkError::DuplicateSymbol`), or if the configured entry point was never added (`LinkError::UndefinedEntry`).

**Examples**

Place two functions in one image, based at a load address, with a chosen entry point:

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
assert!(image.symbol("answer").unwrap() > 0x40_0000);
```

Add a batch of functions in a loop:

```rust
use aot_lang::Compiler;
use ir_lang::{Builder, Type};

let mut compiler = Compiler::new();
for name in ["a", "b", "c"] {
    let mut f = Builder::new(name, &[], Type::Unit);
    f.ret(None);
    compiler.add(&f.finish()).unwrap();
}
let image = compiler.link().unwrap();
assert_eq!(image.symbols().count(), 3);
```

A duplicate function name is a link error:

```rust
use aot_lang::{Compiler, AotError};
use ir_lang::{Builder, Type};
use linker_lang::LinkError;

let make = |name: &str| {
    let mut f = Builder::new(name, &[], Type::Unit);
    f.ret(None);
    f.finish()
};

let mut compiler = Compiler::new();
compiler.add(&make("dup")).unwrap();
compiler.add(&make("dup")).unwrap();

match compiler.link() {
    Err(AotError::Link(LinkError::DuplicateSymbol { name })) => assert_eq!(name, "dup"),
    other => panic!("expected a duplicate-symbol error, got {other:?}"),
}
```

<br>

### `AotError`

```rust
#[non_exhaustive]
pub enum AotError {
    Codegen(codegen_lang::CodegenError),
    Link(linker_lang::LinkError),
}
```

The reason an ahead-of-time compile could not be completed. A compile runs two stages — lower each function, then link — and either can fail. The two variants keep the origin of a failure visible, and each carries the underlying error so the exact cause can be inspected or reported.

`AotError` implements `Display` and [`core::error::Error`], with `source` set to the wrapped error, and converts from both [`CodegenError`](https://docs.rs/codegen-lang) and [`LinkError`](https://docs.rs/linker-lang) so `?` propagates them directly. The enum is `#[non_exhaustive]`, so a `match` on it must include a wildcard arm.

**Variants**

| Variant | Meaning | Common cause |
|---|---|---|
| `Codegen(CodegenError)` | A function could not be lowered to object code. | IR that does not pass `Function::validate`. |
| `Link(LinkError)` | The objects could not be linked into an image. | A duplicate function name, or an undefined entry point. |

**Examples**

Tell which stage failed:

```rust
use aot_lang::{compile, Compiler, AotError};
use ir_lang::{Builder, Type};

// Codegen stage: the function does not validate.
let invalid = Builder::new("bad", &[], Type::Int).finish();
assert!(matches!(compile(&invalid), Err(AotError::Codegen(_))));

// Link stage: a well-formed function, but the requested entry is missing.
let mut good = Builder::new("real", &[], Type::Unit);
good.ret(None);
let mut compiler = Compiler::new();
compiler.entry("nonexistent");
compiler.add(&good.finish()).unwrap();
assert!(matches!(compiler.link(), Err(AotError::Link(_))));
```

Inspect the underlying cause through the `Error` trait:

```rust
use aot_lang::compile;
use core::error::Error;
use ir_lang::{Builder, Type};

let func = Builder::new("bad", &[], Type::Int).finish();
let err = compile(&func).unwrap_err();
assert!(err.source().is_some());
```

<br>

### `Image`

```rust
pub use linker_lang::Image;
```

The linked image a compile produces, re-exported from [`linker-lang`](https://docs.rs/linker-lang). It carries the laid-out `.text` section, the resolved symbol table mapping each function name to its address, and the entry point. A compiled image has one section — `.text` — holding every function's object code end to end.

The `Display` implementation renders a readable link map: the entry point, the section with its address and size, and the symbol table sorted by name.

**Methods** (the ones relevant to a compiled image)

| Method | Returns | Description |
|---|---|---|
| `entry()` | `Option<u64>` | The entry-point address, or `None` if none was configured. |
| `symbol(name)` | `Option<u64>` | The resolved address of the function `name`, or `None`. |
| `symbols()` | `impl Iterator<Item = (&str, u64)>` | The symbol table as `(name, address)` pairs, sorted by name. |
| `section(name)` | `Option<&OutputSection>` | The section named `name` — `.text` for compiled code — or `None`. |
| `sections()` | `&[OutputSection]` | The laid-out sections, in address order. |
| `len()` | `usize` | The number of sections. |
| `is_empty()` | `bool` | Whether the image has no sections. |

**Examples**

Read back the symbols and the entry point:

```rust
use aot_lang::Compiler;
use ir_lang::{Builder, Type};

let mut f = Builder::new("main", &[], Type::Unit);
f.ret(None);

let mut compiler = Compiler::new();
compiler.entry("main");
compiler.add(&f.finish()).unwrap();
let image = compiler.link().unwrap();

assert_eq!(image.entry(), Some(0));
assert_eq!(image.symbol("main"), Some(0));
assert_eq!(image.symbols().count(), 1);
```

Render the link map:

```rust
use aot_lang::compile;
use ir_lang::{Builder, Type};

let mut f = Builder::new("main", &[], Type::Unit);
f.ret(None);
let image = compile(&f.finish()).unwrap();

let map = image.to_string();
assert!(map.contains("entry = 0x0000000000000000"));
assert!(map.contains(".text @ 0x0000000000000000"));
```

<br>

### `OutputSection`

```rust
pub use linker_lang::OutputSection;
```

One laid-out section of an [`Image`](#image), re-exported from [`linker-lang`](https://docs.rs/linker-lang). A compiled image's single `.text` section is the concatenation of every function's object code, placed at the base address, with its bytes final.

**Methods**

| Method | Returns | Description |
|---|---|---|
| `name()` | `&str` | The section's name (`.text` for compiled code). |
| `address()` | `u64` | The address the section begins at (the base address). |
| `data()` | `&[u8]` | The section's final object-code bytes. |
| `len()` | `usize` | The number of bytes. |
| `is_empty()` | `bool` | Whether the section has no bytes. |

**Examples**

```rust
use aot_lang::compile;
use ir_lang::{Builder, BinOp, Type};

let mut b = Builder::new("f", &[Type::Int], Type::Int);
let x = b.block_params(b.entry())[0];
let y = b.bin(BinOp::Add, x, x);
b.ret(Some(y));
let image = compile(&b.finish()).unwrap();

let text = image.section(".text").unwrap();
assert_eq!(text.name(), ".text");
assert_eq!(text.address(), 0);
assert!(!text.data().is_empty());
```

<br>
<hr>
<br>

## Object-code format

Each function in an image's `.text` section is one record, starting at the address its symbol resolves to. Records are laid out back to back in the order functions were added, and each is self-delimiting. This is **format version 1**, written since aot-lang `1.0.1`. All integers are little-endian.

```text
header:
  magic          : 4 bytes  "AOTB"
  version        : u32      1
  register_count : u32      valid registers are 0..register_count
  param_count    : u32
  params         : param_count × u32   each parameter's register, in declaration order
  label_count    : u32
  labels         : label_count × u32   label i resolves to op index labels[i]
  op_count       : u32
body:
  op_count × op-record
```

An op-record is a one-byte tag and its operands. Registers and labels are `u32`. A jump target is a **label id**, resolved through the record's own label table. Label `0` is the entry and resolves to op `0`.

| Tag | Op | Operands |
|---|---|---|
| `0x00` | `const` | `dst:u32`, `kind:u8` (`0` int, `1` float, `2` bool), payload (`i64`, `f64` bit pattern, or a `0`/`1` byte) |
| `0x01` | `bin` | `op:u8`, `dst:u32`, `lhs:u32`, `rhs:u32` |
| `0x02` | `un` | `op:u8`, `dst:u32`, `src:u32` |
| `0x03` | `move` | `dst:u32`, `src:u32` |
| `0x04` | `jump` | `target:u32` (label id) |
| `0x05` | `jump_unless` | `cond:u32`, `target:u32` (label id) |
| `0x06` | `return` | `has_value:u8`, then `value:u32` if `has_value` is `1` |

Binary operator codes: `add 0`, `sub 1`, `mul 2`, `div 3`, `eq 4`, `ne 5`, `lt 6`, `le 7`, `gt 8`, `ge 9`, `and 10`, `or 11`. Unary: `neg 0`, `not 1`.

The record holds every field of the lowered [`codegen_lang::Program`](https://docs.rs/codegen-lang) except its name, which is the symbol. That is the register count, the parameters, the complete label table, and every op, with float constants bit-exact (NaN payloads and the sign of zero survive). The test suite decodes records back out of linked images and compares them field by field with the lowered program. It also runs the decoded bytecode and checks the results against the IR.

A reader must refuse a record whose magic is not `AOTB` or whose version it does not know. aot-lang `1.0.0` wrote an unversioned layout with no magic, no version, and **no label table**. In that layout the targets of `jump` and `jump_unless` could not be resolved, so any function with a branch or loop was unrecoverable. Treat it as version 0; a version-1 reader rejects it at the magic.

<br>
<hr>
<br>

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | on | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`; the flag forwards to every dependency so the whole pipeline drops std together. |
| `serde` | off | Derives `Serialize` / `Deserialize` for [`AotError`](#aoterror) and forwards to the dependencies, so a linked image (and a failure) can be cached, logged, or moved between tools. |

```toml
# no_std build:
aot-lang = { version = "1", default-features = false }

# with serialization:
aot-lang = { version = "1", features = ["serde"] }
```

<br>

## SemVer

As of `1.0.0` the public surface above is frozen. The crate follows [Semantic Versioning](https://semver.org):

- No documented item is removed or changed in a breaking way within `1.x`; breaking changes wait for `2.0`.
- New functionality is additive and arrives in minor releases. Both [`AotError`](#aoterror) and the underlying error types are `#[non_exhaustive]`, so a new failure reason is a minor change, not a breaking one; a `match` on either must keep a wildcard arm.
- The MSRV is Rust `1.85`; raising it is a minor change, never a patch.
- Behaviour is part of the contract: a function that compiles today keeps compiling, the object-code bytes for a given program are stable and identical across hosts **within a format version**, and the link-map `Display` form is unchanged for a given image. The [object-code format](#object-code-format) carries its version. A layout change bumps the version and is called out in the CHANGELOG. `1.0.1` made the one such change so far: version 1 added the header and the label table that `1.0.0`'s unversioned bytes lacked. That was a bug fix, because those bytes could not represent a branch.

This file is updated in lockstep with every release so it always matches the code.

<br>
<hr>

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>
