//! Benchmarks for the ahead-of-time compile pipeline: lowering a function to object
//! code and linking one or many functions into an image.

use aot_lang::{Compiler, compile};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ir_lang::{BinOp, Builder, Function, Type};

/// A straight-line arithmetic function of moderate size: `depth` chained additions
/// over the parameter, ending in a return.
fn arithmetic(name: &str, depth: usize) -> Function {
    let mut b = Builder::new(name, &[Type::Int], Type::Int);
    let mut acc = b.block_params(b.entry())[0];
    for _ in 0..depth {
        let one = b.iconst(1);
        acc = b.bin(BinOp::Add, acc, one);
    }
    b.ret(Some(acc));
    b.finish()
}

/// A counting loop with a back-edge carrying two values — the shape that exercises
/// the control-flow op records (jump, jump_unless, and edge moves).
fn counting_loop(name: &str) -> Function {
    let mut b = Builder::new(name, &[Type::Int], Type::Int);
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

fn bench_compile_single(c: &mut Criterion) {
    let func = arithmetic("arith", 64);
    c.bench_function("compile/arith_depth_64", |b| {
        b.iter(|| compile(black_box(&func)).expect("well-formed"));
    });

    let loop_fn = counting_loop("loop");
    c.bench_function("compile/counting_loop", |b| {
        b.iter(|| compile(black_box(&loop_fn)).expect("well-formed"));
    });
}

fn bench_link_many(c: &mut Criterion) {
    // Sixteen distinct functions linked into one image.
    let funcs: Vec<Function> = (0..16).map(|i| arithmetic(&format!("f{i}"), 16)).collect();

    c.bench_function("link/sixteen_functions", |b| {
        b.iter(|| {
            let mut compiler = Compiler::new();
            for func in &funcs {
                compiler.add(black_box(func)).expect("well-formed");
            }
            compiler.link().expect("no duplicate names")
        });
    });
}

criterion_group!(benches, bench_compile_single, bench_link_many);
criterion_main!(benches);
