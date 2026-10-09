//! Looping programs run correctly end to end through the image.
//!
//! Each test compiles IR with aot-lang, reads the functions back out of the linked
//! image's `.text` section (`support::load`), runs the decoded bytecode, and compares
//! the result with a direct evaluation of the IR (`support::eval`). This covers the
//! whole chain a loader would rely on: codegen-lang's lowering (including the
//! parallel-move fix for permuting back edges in codegen-lang 1.0.1, ISSUES H01),
//! aot-lang's encoding with its label table (ISSUES H02), and the linker layout.

#![allow(clippy::unwrap_used, clippy::expect_used)]

extern crate alloc;

mod support;

use aot_lang::{Compiler, Image};
use ir_lang::{BinOp, Builder, Function, Type};
use proptest::prelude::*;
use support::Value::{self, Int};
use support::generate::{build, build_loop, loop_spec, spec};
use support::{eval, load, run};

/// Runs function `name` from `image` on `args`.
fn run_from_image(image: &Image, name: &str, args: &[Value]) -> Value {
    let loaded = load(image);
    let (_, body, _) = loaded
        .iter()
        .find(|(n, _, _)| n == name)
        .unwrap_or_else(|| panic!("{name} is in the image"));
    run(body, args, 1_000_000).expect("terminates")
}

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

/// `fn fib(n: int) -> int`: iterate `(a, b) = (b, a + b)` `n` times; returns `a`.
/// The back edge passes the header's own parameter `b` into `a`'s slot: a shift.
fn fib() -> Function {
    let mut b = Builder::new("fib", &[Type::Int], Type::Int);
    let n0 = b.block_params(b.entry())[0];
    let header = b.create_block(&[Type::Int, Type::Int, Type::Int]); // (a, b, n)
    let body = b.create_block(&[]);
    let exit = b.create_block(&[]);
    let zero = b.iconst(0);
    let one = b.iconst(1);
    b.jump(header, &[zero, one, n0]);
    b.switch_to(header);
    let h = b.block_params(header).to_vec();
    let z = b.iconst(0);
    let more = b.bin(BinOp::Gt, h[2], z);
    b.branch(more, body, &[], exit, &[]);
    b.switch_to(body);
    let sum = b.bin(BinOp::Add, h[0], h[1]);
    let one2 = b.iconst(1);
    let n2 = b.bin(BinOp::Sub, h[2], one2);
    b.jump(header, &[h[1], sum, n2]);
    b.switch_to(exit);
    b.ret(Some(h[0]));
    b.finish()
}

/// `fn swap(a: int, b: int, n: int) -> int`: `n` times `(a, b) = (b, a)`; returns
/// `a`. The back edge `jump H(b, a, n - 1)` is the swap that codegen-lang 1.0.0
/// miscompiled (both registers ended up holding `b`).
fn swap() -> Function {
    let mut b = Builder::new("swap", &[Type::Int, Type::Int, Type::Int], Type::Int);
    let p = b.block_params(b.entry()).to_vec();
    let header = b.create_block(&[Type::Int, Type::Int, Type::Int]);
    let body = b.create_block(&[]);
    let exit = b.create_block(&[]);
    b.jump(header, &[p[0], p[1], p[2]]);
    b.switch_to(header);
    let h = b.block_params(header).to_vec();
    let zero = b.iconst(0);
    let more = b.bin(BinOp::Gt, h[2], zero);
    b.branch(more, body, &[], exit, &[]);
    b.switch_to(body);
    let one = b.iconst(1);
    let n2 = b.bin(BinOp::Sub, h[2], one);
    b.jump(header, &[h[1], h[0], n2]);
    b.switch_to(exit);
    b.ret(Some(h[0]));
    b.finish()
}

/// `fn rotate(a: int, b: int, c: int, n: int) -> int`: `n` times
/// `(a, b, c) = (b, c, a)`; returns `a * 100 + b * 10 + c`. A 3-cycle back edge.
fn rotate() -> Function {
    let ints = [Type::Int; 4];
    let mut b = Builder::new("rotate", &ints, Type::Int);
    let p = b.block_params(b.entry()).to_vec();
    let header = b.create_block(&ints);
    let body = b.create_block(&[]);
    let exit = b.create_block(&[]);
    b.jump(header, &p);
    b.switch_to(header);
    let h = b.block_params(header).to_vec();
    let zero = b.iconst(0);
    let more = b.bin(BinOp::Gt, h[3], zero);
    b.branch(more, body, &[], exit, &[]);
    b.switch_to(body);
    let one = b.iconst(1);
    let n2 = b.bin(BinOp::Sub, h[3], one);
    b.jump(header, &[h[1], h[2], h[0], n2]);
    b.switch_to(exit);
    let hundred = b.iconst(100);
    let ten = b.iconst(10);
    let a100 = b.bin(BinOp::Mul, h[0], hundred);
    let b10 = b.bin(BinOp::Mul, h[1], ten);
    let s = b.bin(BinOp::Add, a100, b10);
    let r = b.bin(BinOp::Add, s, h[2]);
    b.ret(Some(r));
    b.finish()
}

/// All four looping functions in one image, at a nonzero base address.
fn loops_image() -> Image {
    let mut compiler = Compiler::new();
    compiler.base_address(0x40_0000).entry("fib");
    for func in [sum_to(), fib(), swap(), rotate()] {
        compiler.add(&func).unwrap();
    }
    compiler.link().unwrap()
}

#[test]
fn test_sum_to_runs_from_the_image() {
    let image = loops_image();
    for n in [0, 1, 5, 100, -3] {
        let expected = if n > 0 { n * (n + 1) / 2 } else { 0 };
        assert_eq!(run_from_image(&image, "sum_to", &[Int(n)]), Int(expected));
    }
}

#[test]
fn test_fib_runs_from_the_image() {
    let image = loops_image();
    let want = [0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55];
    for (n, &f) in want.iter().enumerate() {
        assert_eq!(run_from_image(&image, "fib", &[Int(n as i64)]), Int(f));
    }
    assert_eq!(
        run_from_image(&image, "fib", &[Int(90)]),
        Int(2_880_067_194_370_816_120)
    );
}

#[test]
fn test_swap_back_edge_runs_from_the_image() {
    // Even trip counts return a, odd ones b. Under codegen-lang 1.0.0's sequential
    // moves every trip count >= 1 returned b.
    let image = loops_image();
    for n in 0..6 {
        let want = if n % 2 == 0 { 7 } else { 9 };
        assert_eq!(
            run_from_image(&image, "swap", &[Int(7), Int(9), Int(n)]),
            Int(want),
            "n = {n}"
        );
    }
}

#[test]
fn test_rotate_back_edge_runs_from_the_image() {
    let image = loops_image();
    let want = [123, 231, 312, 123, 231];
    for (n, &w) in want.iter().enumerate() {
        assert_eq!(
            run_from_image(&image, "rotate", &[Int(1), Int(2), Int(3), Int(n as i64)]),
            Int(w),
            "n = {n}"
        );
    }
}

#[test]
fn test_every_loop_matches_the_ir_evaluation() {
    let image = loops_image();
    let funcs = [sum_to(), fib(), swap(), rotate()];
    let inputs: [&[i64]; 4] = [&[0, 1, 7, 40], &[0, 1, 2, 30], &[0, 1, 2, 3], &[0, 1, 2, 4]];
    for (func, ns) in funcs.iter().zip(inputs) {
        for &n in ns {
            let args: Vec<Value> = match func.params().len() {
                1 => vec![Int(n)],
                3 => vec![Int(-5), Int(11), Int(n)],
                _ => vec![Int(4), Int(5), Int(6), Int(n)],
            };
            assert_eq!(
                Some(run_from_image(&image, func.name(), &args)),
                eval(func, &args, 100_000),
                "{}({args:?})",
                func.name()
            );
        }
    }
}

/// The generator really produces the shapes the properties are meant to cover.
#[test]
fn test_generated_graphs_contain_loops() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let mut with_backward_jump = 0;
    let samples = 256;
    for _ in 0..samples {
        let s = spec().new_tree(&mut runner).unwrap().current();
        let image = aot_lang::compile(&build("f", &s)).unwrap();
        let (_, body, _) = &load(&image)[0];
        let backward = body.ops.iter().enumerate().any(|(at, op)| match *op {
            codegen_lang::Op::Jump { target } | codegen_lang::Op::JumpUnless { target, .. } => {
                body.label_offset(target).is_some_and(|to| to <= at)
            }
            _ => false,
        });
        if backward {
            with_backward_jump += 1;
        }
    }
    assert!(
        with_backward_jump * 5 >= samples,
        "only {with_backward_jump} of {samples} generated graphs loop"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Counted loops whose back edge permutes, rotates, duplicates, and offsets its
    /// variables, run from the image for 0 to 12 trips, agree with the IR evaluation.
    /// This is the shape codegen-lang 1.0.0 miscompiled (ISSUES H01); under 1.0.0
    /// this property fails.
    #[test]
    fn permuting_loops_run_from_the_image_like_the_ir(s in loop_spec(), trips in 0i64..=12) {
        let func = build_loop("l", &s);
        let args = [Int(trips)];
        let expected = eval(&func, &args, 1_000).expect("a counted loop ends");
        let image = aot_lang::compile(&func).unwrap();
        let (_, body, _) = &load(&image)[0];
        prop_assert_eq!(run(body, &args, 100_000), Some(expected));
    }

    /// Arbitrary control-flow graphs, compiled to an image and run from the decoded
    /// bytes, agree with the IR evaluation whenever the IR evaluation terminates.
    #[test]
    fn arbitrary_programs_run_from_the_image_like_the_ir(
        s in spec(),
        a in -3i64..=3, b in any::<i64>(), c in -2i64..=40,
    ) {
        let func = build("f", &s);
        let args: Vec<Value> = [a, b, c][..s.params].iter().map(|&v| Int(v)).collect();
        let Some(expected) = eval(&func, &args, 2_000) else {
            // A generated loop that does not end within the budget: nothing to compare.
            return Ok(());
        };
        let image = aot_lang::compile(&func).unwrap();
        let (_, body, _) = &load(&image)[0];
        // Each block lowers to a bounded number of ops, so the IR's block budget
        // bounds the bytecode's op count.
        prop_assert_eq!(run(body, &args, 2_000 * 64), Some(expected));
    }
}
