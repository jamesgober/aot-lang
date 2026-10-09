//! Shared integration-test support: read functions back out of a linked [`Image`],
//! run their bytecode, and check it against a direct evaluation of the IR.
//!
//! - [`decode`] reads the object-code format (also compiled into the unit tests).
//! - [`load`] finds each function's record in the image's `.text` section by its
//!   symbol and decodes it.
//! - [`run`] executes a decoded body; [`eval`] evaluates the source IR. Both share
//!   one set of operator semantics ([`binary`], [`unary`]; codegen-lang re-exports
//!   ir-lang's operator enums, so one function serves both), replicated from
//!   codegen-lang's test-support reference interpreter (wrapping integers), so the
//!   two can only disagree about control and data flow, which is what the format and
//!   the lowering must preserve. Integer division by zero yields `0` here only so
//!   generated programs stay total; it is a test artifact, not a statement about
//!   bytecode semantics.
//! - [`gen`] generates arbitrary valid control-flow graphs.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    reason = "test-support code: each test crate uses a subset, and a bad value should fail loudly"
)]

pub mod decode;
pub mod generate;

use aot_lang::Image;
use codegen_lang::{BinOp, Const, Op, Reg, UnOp};
use decode::{Body, decode_prefix};
use ir_lang::{Function, Inst, Terminator, Value as IrValue};

/// A runtime value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    /// A signed integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// The absence of a value.
    Unit,
}

impl Value {
    fn boolean(self) -> bool {
        match self {
            Value::Bool(v) => v,
            other => panic!("expected a bool, found {other:?}"),
        }
    }
}

/// Every function in `image`, decoded from its `.text` record: `(name, body,
/// record length)`, in address order. Panics if a symbol does not point at a valid
/// record, or if the records do not tile the section exactly.
#[must_use]
pub fn load(image: &Image) -> Vec<(String, Body, usize)> {
    let Some(text) = image.section(".text") else {
        assert_eq!(image.symbols().count(), 0, "symbols but no .text section");
        return Vec::new();
    };
    let mut symbols: Vec<(&str, u64)> = image.symbols().collect();
    symbols.sort_by_key(|&(_, addr)| addr);

    let mut out = Vec::new();
    let mut expected_start = 0usize;
    for (name, addr) in symbols {
        let start = usize::try_from(addr - text.address()).unwrap();
        assert_eq!(start, expected_start, "{name}: records are contiguous");
        let (body, used) = decode_prefix(&text.data()[start..])
            .unwrap_or_else(|err| panic!("{name} at {start}: {err:?}"));
        expected_start = start + used;
        out.push((name.to_string(), body, used));
    }
    assert_eq!(expected_start, text.len(), "records tile .text exactly");
    out
}

/// Runs a decoded body on `args`. Returns `None` if it executes more than `fuel`
/// ops. Panics on a malformed body (an out-of-range register or label), which would
/// itself be a bug the test should surface.
#[must_use]
pub fn run(body: &Body, args: &[Value], fuel: usize) -> Option<Value> {
    let mut regs = vec![Value::Unit; body.register_count as usize];
    assert_eq!(body.params.len(), args.len(), "argument count mismatch");
    for (&Reg(r), &arg) in body.params.iter().zip(args) {
        regs[r as usize] = arg;
    }
    let reg = |r: Reg| r.0 as usize;
    let mut pc = body
        .label_offset(codegen_lang::Label(0))
        .expect("entry label");
    let mut remaining = fuel;
    loop {
        remaining = remaining.checked_sub(1)?;
        match body.ops[pc] {
            Op::Const { dst, value } => {
                regs[reg(dst)] = match value {
                    Const::Int(v) => Value::Int(v),
                    Const::Float(v) => Value::Float(v),
                    Const::Bool(v) => Value::Bool(v),
                };
                pc += 1;
            }
            Op::Bin { op, dst, lhs, rhs } => {
                regs[reg(dst)] = binary(op, regs[reg(lhs)], regs[reg(rhs)]);
                pc += 1;
            }
            Op::Un { op, dst, src } => {
                regs[reg(dst)] = unary(op, regs[reg(src)]);
                pc += 1;
            }
            Op::Move { dst, src } => {
                regs[reg(dst)] = regs[reg(src)];
                pc += 1;
            }
            Op::Jump { target } => pc = body.label_offset(target).expect("jump target"),
            Op::JumpUnless { cond, target } => {
                if regs[reg(cond)].boolean() {
                    pc += 1;
                } else {
                    pc = body.label_offset(target).expect("jump target");
                }
            }
            Op::Return { value } => return Some(value.map_or(Value::Unit, |r| regs[reg(r)])),
        }
    }
}

/// Evaluates `func` directly on `args`. Returns `None` if it enters more than `fuel`
/// blocks. Block arguments bind as one parallel assignment, as SSA defines.
#[must_use]
pub fn eval(func: &Function, args: &[Value], fuel: usize) -> Option<Value> {
    let mut env: Vec<Option<Value>> = vec![None; func.value_count()];
    let read = |env: &[Option<Value>], v: IrValue| env[v.index()].expect("defined before use");
    let mut block = func.entry();
    for (&param, &arg) in func.block_params(block).iter().zip(args) {
        env[param.index()] = Some(arg);
    }
    let mut remaining = fuel;
    loop {
        remaining = remaining.checked_sub(1)?;
        for &value in func.insts(block) {
            let result = match *func.inst(value).expect("listed instruction") {
                Inst::Iconst(v) => Value::Int(v),
                Inst::Fconst(v) => Value::Float(v),
                Inst::Bconst(v) => Value::Bool(v),
                Inst::Bin(op, l, r) => binary(op, read(&env, l), read(&env, r)),
                Inst::Un(op, x) => unary(op, read(&env, x)),
            };
            env[value.index()] = Some(result);
        }
        let (target, args) = match func.terminator(block).expect("terminated") {
            Terminator::Return(v) => return Some(v.map_or(Value::Unit, |v| read(&env, v))),
            Terminator::Jump(target, args) => (*target, args),
            Terminator::Branch {
                cond,
                then_block,
                then_args,
                else_block,
                else_args,
            } => {
                if read(&env, *cond).boolean() {
                    (*then_block, then_args)
                } else {
                    (*else_block, else_args)
                }
            }
        };
        let incoming: Vec<Value> = args.iter().map(|&a| read(&env, a)).collect();
        for (&param, value) in func.block_params(target).iter().zip(incoming) {
            env[param.index()] = Some(value);
        }
        block = target;
    }
}

/// The shared binary-operator semantics (wrapping integers; see the module docs).
#[must_use]
pub fn binary(op: BinOp, lhs: Value, rhs: Value) -> Value {
    match (op, lhs, rhs) {
        (BinOp::Eq, a, b) => Value::Bool(a == b),
        (BinOp::Ne, a, b) => Value::Bool(a != b),
        (BinOp::And, a, b) => Value::Bool(a.boolean() && b.boolean()),
        (BinOp::Or, a, b) => Value::Bool(a.boolean() || b.boolean()),
        (_, Value::Int(a), Value::Int(b)) => match op {
            BinOp::Add => Value::Int(a.wrapping_add(b)),
            BinOp::Sub => Value::Int(a.wrapping_sub(b)),
            BinOp::Mul => Value::Int(a.wrapping_mul(b)),
            BinOp::Div => Value::Int(if b == 0 { 0 } else { a.wrapping_div(b) }),
            BinOp::Lt => Value::Bool(a < b),
            BinOp::Le => Value::Bool(a <= b),
            BinOp::Gt => Value::Bool(a > b),
            BinOp::Ge => Value::Bool(a >= b),
            _ => unreachable!(),
        },
        (_, Value::Float(a), Value::Float(b)) => match op {
            BinOp::Add => Value::Float(a + b),
            BinOp::Sub => Value::Float(a - b),
            BinOp::Mul => Value::Float(a * b),
            BinOp::Div => Value::Float(a / b),
            BinOp::Lt => Value::Bool(a < b),
            BinOp::Le => Value::Bool(a <= b),
            BinOp::Gt => Value::Bool(a > b),
            BinOp::Ge => Value::Bool(a >= b),
            _ => unreachable!(),
        },
        other => panic!("ill-typed operands: {other:?}"),
    }
}

/// The shared unary-operator semantics.
#[must_use]
pub fn unary(op: UnOp, operand: Value) -> Value {
    match (op, operand) {
        (UnOp::Neg, Value::Int(v)) => Value::Int(v.wrapping_neg()),
        (UnOp::Neg, Value::Float(v)) => Value::Float(-v),
        (UnOp::Not, v) => Value::Bool(!v.boolean()),
        other => panic!("ill-typed operand: {other:?}"),
    }
}
