//! End-to-end tests over the public surface: build IR, compile it ahead of time,
//! and inspect the resulting image.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use aot_lang::{AotError, Compiler, compile};
use ir_lang::{BinOp, Builder, Function, Type};

/// `fn sum_to(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }`
/// — a loop with a back-edge, so compiling it drives the control-flow lowering all
/// the way through the encoder.
fn sum_to() -> Function {
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
    b.finish()
}

fn constant(name: &str, value: i64) -> Function {
    let mut b = Builder::new(name, &[], Type::Int);
    let c = b.iconst(value);
    b.ret(Some(c));
    b.finish()
}

#[test]
fn compiles_a_loop_to_a_single_entry_image() {
    let image = compile(&sum_to()).unwrap();

    assert_eq!(image.entry(), Some(0));
    assert_eq!(image.symbol("sum_to"), Some(0));
    assert_eq!(image.len(), 1);
    let text = image.section(".text").unwrap();
    assert_eq!(text.name(), ".text");
    assert!(!text.is_empty());
}

#[test]
fn lays_out_three_functions_end_to_end() {
    let mut compiler = Compiler::new();
    compiler.base_address(0x10_0000).entry("two");
    compiler.add(&constant("one", 1)).unwrap();
    compiler.add(&constant("two", 2)).unwrap();
    compiler.add(&constant("three", 3)).unwrap();

    let image = compiler.link().unwrap();

    assert_eq!(image.entry(), Some(image.symbol("two").unwrap()));
    assert_eq!(image.symbol("one"), Some(0x10_0000));
    // Addresses are strictly increasing in add order, all inside the one section.
    let one = image.symbol("one").unwrap();
    let two = image.symbol("two").unwrap();
    let three = image.symbol("three").unwrap();
    assert!(one < two && two < three);
    assert_eq!(image.symbols().count(), 3);
}

#[test]
fn empty_compiler_links_to_an_empty_image() {
    let image = Compiler::new().link().unwrap();
    assert!(image.is_empty());
    assert_eq!(image.entry(), None);
}

#[test]
fn reports_which_stage_failed() {
    // Codegen stage: the function does not validate.
    let invalid = Builder::new("bad", &[], Type::Int).finish();
    assert!(matches!(compile(&invalid), Err(AotError::Codegen(_))));

    // Link stage: a well-formed function, but the requested entry is not present.
    let mut compiler = Compiler::new();
    compiler.entry("nonexistent");
    compiler.add(&constant("real", 0)).unwrap();
    assert!(matches!(compiler.link(), Err(AotError::Link(_))));
}

#[test]
fn image_bytes_are_reproducible_across_hosts() {
    // The object code is fixed little-endian, so recompiling yields identical bytes.
    let first = compile(&sum_to()).unwrap();
    let second = compile(&sum_to()).unwrap();
    assert_eq!(
        first.section(".text").unwrap().data(),
        second.section(".text").unwrap().data()
    );
}

#[test]
fn base_address_shifts_every_symbol_uniformly() {
    // The base address is layout only: it offsets every symbol by the same amount
    // and leaves the object-code bytes untouched.
    let build = || {
        let mut compiler = Compiler::new();
        compiler.add(&constant("a", 1)).unwrap();
        compiler.add(&constant("b", 2)).unwrap();
        compiler.add(&constant("c", 3)).unwrap();
        compiler
    };

    let base = 0x0040_0000;
    let at_zero = build().link().unwrap();
    let shifted = {
        let mut compiler = build();
        compiler.base_address(base);
        compiler.link().unwrap()
    };

    for name in ["a", "b", "c"] {
        let low = at_zero.symbol(name).unwrap();
        let high = shifted.symbol(name).unwrap();
        assert_eq!(
            high,
            low + base,
            "{name} shifts by exactly the base address"
        );
    }
    // The code itself is identical regardless of where it is placed.
    assert_eq!(
        at_zero.section(".text").unwrap().data(),
        shifted.section(".text").unwrap().data()
    );
}

#[test]
fn links_many_functions_with_a_complete_sorted_symbol_table() {
    let mut compiler = Compiler::new();
    for i in 0..64 {
        // Names are zero-padded so lexical order matches numeric order.
        compiler.add(&constant(&format!("f{i:02}"), i)).unwrap();
    }
    let image = compiler.link().unwrap();

    assert_eq!(image.symbols().count(), 64);
    // `symbols()` is sorted by name; addresses are strictly increasing in that order
    // because add order and name order coincide here.
    let mut previous: Option<u64> = None;
    for (_name, address) in image.symbols() {
        if let Some(prev) = previous {
            assert!(address > prev, "addresses strictly increase");
        }
        previous = Some(address);
    }
}

#[test]
fn function_names_with_punctuation_become_symbols() {
    // A front-end may mangle names with dots or underscores; they pass through as
    // ordinary symbol names.
    let mut compiler = Compiler::new();
    compiler.add(&constant("core::math::add", 0)).unwrap();
    compiler.add(&constant("_start", 0)).unwrap();
    let image = compiler.link().unwrap();

    assert!(image.symbol("core::math::add").is_some());
    assert!(image.symbol("_start").is_some());
}
