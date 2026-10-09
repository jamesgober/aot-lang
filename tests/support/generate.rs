//! Arbitrary valid control-flow graphs for the format and execution properties.
//!
//! A [`Spec`] is plain data that proptest generates and shrinks; [`build`] turns it
//! into an `ir_lang::Function` that always validates. Validity is by construction:
//! a block only uses its own parameters, its own instructions, and constants, so no
//! use depends on dominance, and every edge passes exactly as many `int` arguments as
//! its target declares. Edges may go to any non-entry block, backwards included, so
//! the graphs have loops whose back edges pass the header's own parameters in any
//! order (swaps, rotations, duplications): the shapes codegen-lang 1.0.1's
//! parallel-move fix (ISSUES H01) is about, and the shapes whose jump targets
//! aot-lang 1.0.0 could not encode (ISSUES H02).

use ir_lang::{BinOp, Block, Builder, Function, Type, Value};
use proptest::prelude::*;

/// One block's shape.
#[derive(Clone, Debug)]
pub struct BlockSpec {
    /// Number of `int` parameters (the entry block uses the function's instead).
    pub params: usize,
    /// Instructions: `(kind, a, b, constant)`. `kind` picks a constant or an
    /// add/sub/mul/div of two earlier local values (`a`/`b` index them modulo).
    pub insts: Vec<(u8, usize, usize, i64)>,
    /// The terminator: `kind` picks return / jump / branch; the targets index the
    /// non-entry blocks modulo their count; the argument selectors index local values.
    pub term: (u8, usize, usize, usize, Vec<usize>, Vec<usize>),
}

/// A whole function: parameter count and blocks (block 0 is the entry).
#[derive(Clone, Debug)]
pub struct Spec {
    /// Number of `int` parameters, 1 to 3.
    pub params: usize,
    /// The blocks; at least one.
    pub blocks: Vec<BlockSpec>,
}

fn edge_i64() -> impl Strategy<Value = i64> {
    prop_oneof![-4i64..=4, Just(i64::MIN), Just(i64::MAX), any::<i64>(),]
}

fn block_spec() -> impl Strategy<Value = BlockSpec> {
    (
        0usize..=3,
        prop::collection::vec((0u8..6, any::<usize>(), any::<usize>(), edge_i64()), 0..=5),
        (
            0u8..10,
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
            prop::collection::vec(any::<usize>(), 3),
            prop::collection::vec(any::<usize>(), 3),
        ),
    )
        .prop_map(|(params, insts, term)| BlockSpec {
            params,
            insts,
            term,
        })
}

/// Specs of 1 to 6 blocks over 1 to 3 parameters.
pub fn spec() -> impl Strategy<Value = Spec> {
    (1usize..=3, prop::collection::vec(block_spec(), 1..=6))
        .prop_map(|(params, blocks)| Spec { params, blocks })
}

/// Builds the function `name` a spec describes. Always valid.
#[must_use]
pub fn build(name: &str, spec: &Spec) -> Function {
    let mut b = Builder::new(name, &vec![Type::Int; spec.params], Type::Int);
    let entry = b.entry();
    let mut blocks: Vec<Block> = vec![entry];
    for block in &spec.blocks[1..] {
        blocks.push(b.create_block(&vec![Type::Int; block.params]));
    }
    // Non-entry blocks are the only jump targets.
    let targets: Vec<Block> = blocks[1..].to_vec();

    for (i, block) in spec.blocks.iter().enumerate() {
        let here = blocks[i];
        b.switch_to(here);
        let mut local: Vec<Value> = b.block_params(here).to_vec();
        if local.is_empty() {
            local.push(b.iconst(0));
        }
        for &(kind, x, y, k) in &block.insts {
            let lhs = local[x % local.len()];
            let rhs = local[y % local.len()];
            let v = match kind {
                0 | 1 => b.iconst(k),
                2 => b.bin(BinOp::Add, lhs, rhs),
                3 => b.bin(BinOp::Sub, lhs, rhs),
                4 => b.bin(BinOp::Mul, lhs, rhs),
                _ => b.bin(BinOp::Div, lhs, rhs),
            };
            local.push(v);
        }

        let (kind, t1, t2, c, ref a1, ref a2) = block.term;
        let pick = |sel: &[usize], target: Block, b: &Builder| -> Vec<Value> {
            let n = b.block_params(target).len();
            (0..n).map(|j| local[sel[j] % local.len()]).collect()
        };
        if targets.is_empty() || kind < 2 {
            let v = local[c % local.len()];
            b.ret(Some(v));
        } else if kind < 6 {
            let target = targets[t1 % targets.len()];
            let args = pick(a1, target, &b);
            b.jump(target, &args);
        } else {
            let then_b = targets[t1 % targets.len()];
            let else_b = targets[t2 % targets.len()];
            let then_a = pick(a1, then_b, &b);
            let else_a = pick(a2, else_b, &b);
            let x = local[c % local.len()];
            let y = local[(c / 7) % local.len()];
            let cond = b.bin(BinOp::Lt, x, y);
            b.branch(cond, then_b, &then_a, else_b, &else_a);
        }
    }
    b.finish()
}

/// A counted loop whose back edge rebinds its variables arbitrarily: each new value
/// is an old variable (any one, so permutations, rotations, and duplications all
/// occur) optionally plus a constant. The loop runs exactly `n` times for the
/// function argument `n`, so every trip is observable in the result.
#[derive(Clone, Debug)]
pub struct LoopSpec {
    /// Initial values of the loop variables (2 to 5 of them).
    pub init: Vec<i64>,
    /// For each variable, `(source variable, add constant?, constant)`.
    pub next: Vec<(usize, bool, i64)>,
}

/// Loop specs over 2 to 5 variables.
pub fn loop_spec() -> impl Strategy<Value = LoopSpec> {
    (2usize..=5).prop_flat_map(|n| {
        (
            prop::collection::vec(-50i64..=50, n),
            prop::collection::vec((0..n, any::<bool>(), -3i64..=3), n),
        )
            .prop_map(|(init, next)| LoopSpec { init, next })
    })
}

/// Builds `fn name(n: int) -> int` for a loop spec. The result folds every variable
/// into one value position-sensitively, so a wrong permutation changes it.
#[must_use]
pub fn build_loop(name: &str, spec: &LoopSpec) -> Function {
    let vars = spec.init.len();
    let mut b = Builder::new(name, &[Type::Int], Type::Int);
    let n = b.block_params(b.entry())[0];
    let header = b.create_block(&vec![Type::Int; vars + 1]);
    let body = b.create_block(&[]);
    let exit = b.create_block(&[]);

    let mut args: Vec<Value> = spec.init.iter().map(|&v| b.iconst(v)).collect();
    args.push(n);
    b.jump(header, &args);

    b.switch_to(header);
    let h = b.block_params(header).to_vec();
    let zero = b.iconst(0);
    let more = b.bin(BinOp::Gt, h[vars], zero);
    b.branch(more, body, &[], exit, &[]);

    b.switch_to(body);
    let mut next = Vec::with_capacity(vars + 1);
    for &(src, add, k) in &spec.next {
        let old = h[src % vars];
        next.push(if add {
            let c = b.iconst(k);
            b.bin(BinOp::Add, old, c)
        } else {
            old
        });
    }
    let one = b.iconst(1);
    next.push(b.bin(BinOp::Sub, h[vars], one));
    b.jump(header, &next);

    b.switch_to(exit);
    let base = b.iconst(1_000_003);
    let mut acc = b.iconst(0);
    for &v in &h[..vars] {
        let scaled = b.bin(BinOp::Mul, acc, base);
        acc = b.bin(BinOp::Add, scaled, v);
    }
    b.ret(Some(acc));
    b.finish()
}
