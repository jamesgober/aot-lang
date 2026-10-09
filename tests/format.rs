//! The object-code format round-trips through a linked image (ISSUES H02).
//!
//! `1.0.0` never wrote a program's label table, so the jump targets in its bytes
//! could not be resolved and any function with a branch was unrecoverable from the
//! image. These tests read every function back out of the image's `.text` section
//! with the tests-only decoder in `support/decode.rs` and compare it with the
//! program codegen-lang lowered: name (from the symbol table), register count,
//! parameters, the full label table, and every op.

#![allow(clippy::unwrap_used, clippy::expect_used)]

extern crate alloc;

mod support;

use aot_lang::{Compiler, compile};
use ir_lang::{BinOp, Builder, Function, Type};
use proptest::prelude::*;
use support::decode::{DecodeError, body_of, decode_exact};
use support::generate::{build, spec};
use support::load;

/// `fn sum_to(n: int) -> int { let mut acc = 0; while n > 0 { acc += n; n -= 1 } acc }`
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

#[test]
fn test_a_looping_function_round_trips_through_the_image() {
    let func = sum_to();
    let program = codegen_lang::compile(&func).unwrap();
    let image = compile(&func).unwrap();
    let loaded = load(&image);
    assert_eq!(loaded.len(), 1);
    let (name, body, _) = &loaded[0];
    assert_eq!(name, program.name());
    assert_eq!(*body, body_of(&program));
    // The loop needs its label table: there is a backward jump, and its target
    // resolves to an op before the jump.
    let backward = body.ops.iter().enumerate().any(|(at, op)| {
        matches!(*op, codegen_lang::Op::Jump { target }
            if body.label_offset(target).is_some_and(|to| to < at))
    });
    assert!(backward, "sum_to lowers to a backward jump");
}

#[test]
fn test_the_section_bytes_of_one_function_decode_exactly() {
    let image = compile(&sum_to()).unwrap();
    let text = image.section(".text").unwrap();
    assert_eq!(&text.data()[..4], b"AOTB");
    assert!(decode_exact(text.data()).is_ok());
}

/// `fn constant() -> int { value }`, named `name`.
fn constant(name: &str, value: i64) -> Function {
    let mut b = Builder::new(name, &[], Type::Int);
    let c = b.iconst(value);
    b.ret(Some(c));
    b.finish()
}

#[test]
fn test_every_function_in_a_multi_function_image_round_trips() {
    let funcs = [
        constant("zero", 0),
        sum_to(),
        constant("min", i64::MIN),
        constant("max", i64::MAX),
    ];
    let mut compiler = Compiler::new();
    compiler.base_address(0x1000);
    for func in &funcs {
        compiler.add(func).unwrap();
    }
    let loaded = load(&compiler.link().unwrap());
    assert_eq!(loaded.len(), funcs.len());
    // Laid out in add order; `load` returns them in address order.
    for ((name, body, _), func) in loaded.iter().zip(&funcs) {
        let program = codegen_lang::compile(func).unwrap();
        assert_eq!(name, program.name());
        assert_eq!(*body, body_of(&program), "{name}");
    }
}

#[test]
fn test_bytes_in_the_1_0_0_layout_are_refused() {
    // 1.0.0 began a record with its register count. For sum_to that is a small
    // number, never the magic, so a 1.0.1 reader refuses the bytes outright instead
    // of misreading them.
    let mut old = Vec::new();
    old.extend_from_slice(&5u32.to_le_bytes()); // register_count
    old.extend_from_slice(&1u32.to_le_bytes()); // param_count
    old.extend_from_slice(&0u32.to_le_bytes()); // params[0]
    old.extend_from_slice(&0u32.to_le_bytes()); // op_count
    assert_eq!(decode_exact(&old), Err(DecodeError::BadMagic));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary control-flow graphs, several to an image at an arbitrary base
    /// address: every function read back from the image equals the program
    /// codegen-lang lowered, in full, and the records tile `.text` exactly.
    #[test]
    fn arbitrary_programs_round_trip_through_the_image(
        specs in prop::collection::vec(spec(), 1..=4),
        base in any::<u32>(),
    ) {
        let mut compiler = Compiler::new();
        compiler.base_address(u64::from(base));
        let mut expected = Vec::new();
        for (i, s) in specs.iter().enumerate() {
            let func = build(&format!("f{i}"), s);
            compiler.add(&func).unwrap();
            let program = codegen_lang::compile(&func).unwrap();
            expected.push((program.name().to_string(), body_of(&program)));
        }
        let image = compiler.link().unwrap();
        let loaded = load(&image);
        prop_assert_eq!(loaded.len(), expected.len());
        for ((name, body, _), (want_name, want_body)) in loaded.iter().zip(&expected) {
            // Laid out in add order, and symbols sort by address, so the orders match.
            prop_assert_eq!(name, want_name);
            prop_assert_eq!(body, want_body);
        }
    }

    /// Every jump target in a decoded record resolves through its own label table
    /// to an op inside the record.
    #[test]
    fn every_decoded_jump_target_resolves(s in spec()) {
        let image = compile(&build("f", &s)).unwrap();
        for (_, body, _) in load(&image) {
            for op in &body.ops {
                if let codegen_lang::Op::Jump { target } | codegen_lang::Op::JumpUnless { target, .. } = *op {
                    let at = body.label_offset(target);
                    prop_assert!(at.is_some_and(|at| at < body.ops.len()), "{:?}", op);
                }
            }
        }
    }
}
