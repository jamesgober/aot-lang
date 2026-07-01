//! # aot_lang
//!
//! Ahead-of-time compile the [`ir-lang`](ir_lang) intermediate representation into a
//! single linked image.
//!
//! aot-lang is the batch counterpart to a just-in-time compiler: instead of running
//! a function the moment it is generated, it lowers one or more functions to object
//! code and lays them out together into an [`Image`] — a self-contained artifact
//! with its code, a symbol table, and an entry point, ready to be inspected,
//! serialized, or loaded. It is the end of the pipeline a front-end follows: parse,
//! type-check, lower to IR, and then compile the whole program ahead of time.
//!
//! ## The pipeline
//!
//! A compile is two stages, one per dependency it wires:
//!
//! 1. **Lower.** Each [`ir_lang::Function`] is compiled to a
//!    [`codegen_lang::Program`] of register bytecode — the object code — after being
//!    checked with `Function::validate`, so only well-formed SSA is ever lowered.
//! 2. **Encode and link.** Each program is encoded into the bytes of a `.text`
//!    section with a symbol at its start, and [`linker-lang`](linker_lang) places
//!    the objects at addresses, resolves the entry point, and produces the [`Image`].
//!
//! The object-code byte format is little-endian on every target, so an image
//! compiled on one host is byte-identical on another.
//!
//! ## Surface
//!
//! The surface is small. [`compile`] is the shortcut for a single function; it
//! returns an image with that function as the entry point. [`Compiler`] is the
//! builder for several functions in one image, and for choosing the base address
//! and entry point. [`AotError`] is the failure of either stage. The result type,
//! [`Image`], and its [`OutputSection`]s are re-exported from `linker-lang`.
//!
//! ## Example
//!
//! Compile two functions into one image laid out at a load address:
//!
//! ```
//! use aot_lang::Compiler;
//! use ir_lang::{Builder, BinOp, Type};
//!
//! // fn add(a: int, b: int) -> int { a + b }
//! let mut add = Builder::new("add", &[Type::Int, Type::Int], Type::Int);
//! let a = add.block_params(add.entry())[0];
//! let b = add.block_params(add.entry())[1];
//! let sum = add.bin(BinOp::Add, a, b);
//! add.ret(Some(sum));
//!
//! // fn answer() -> int { 42 }
//! let mut answer = Builder::new("answer", &[], Type::Int);
//! let c = answer.iconst(42);
//! answer.ret(Some(c));
//!
//! let mut compiler = Compiler::new();
//! compiler.base_address(0x40_0000).entry("add");
//! compiler.add(&add.finish()).unwrap();
//! compiler.add(&answer.finish()).unwrap();
//! let image = compiler.link().unwrap();
//!
//! assert_eq!(image.entry(), Some(0x40_0000));
//! assert_eq!(image.symbol("add"), Some(0x40_0000));
//! assert!(image.symbol("answer").is_some());
//! ```
//!
//! ## No-std
//!
//! The crate needs only `alloc`; it performs no I/O and touches no operating-system
//! facility. Build with `default-features = false` to drop the standard library. The
//! `serde` feature derives serialization for [`AotError`] and forwards to the
//! dependencies so a linked image round-trips.
//!
//! ## Stability
//!
//! The public surface is being designed across the `0.x` series and frozen at
//! `1.0`. See [`docs/API.md`](https://github.com/jamesgober/aot-lang/blob/main/docs/API.md)
//! and [`dev/ROADMAP.md`](https://github.com/jamesgober/aot-lang/blob/main/dev/ROADMAP.md).

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(unused_must_use)]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr
)]

extern crate alloc;

mod compiler;
mod encode;
mod error;

pub use compiler::{Compiler, compile};
pub use error::AotError;

/// The linked image produced by a compile, re-exported from
/// [`linker-lang`](linker_lang). It carries the laid-out code sections, the resolved
/// symbol table, and the entry point.
pub use linker_lang::Image;
/// One laid-out section of an [`Image`], re-exported from
/// [`linker-lang`](linker_lang). A compiled image has a single `.text` section
/// holding every function's object code.
pub use linker_lang::OutputSection;
