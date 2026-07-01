//! The object-code format: how a lowered [`Program`] becomes the bytes of a
//! linkable section.
//!
//! A [`codegen_lang::Program`] is a name, a list of parameter registers, a register
//! count, and a stream of [`Op`]s. [`encode`] flattens everything except the name
//! into a compact little-endian byte string; the name lives in the object's symbol
//! table, not its bytes, because that is what the linker resolves an address for.
//!
//! ## Layout
//!
//! All integers are little-endian. The bytes are a fixed header followed by one
//! record per op:
//!
//! ```text
//! header:
//!   register_count : u32
//!   param_count    : u32
//!   params         : param_count × u32   (each parameter's register number)
//!   op_count       : u32
//! body:
//!   op_count × op-record
//! ```
//!
//! An op-record is a one-byte tag and its operands. Register and label operands are
//! `u32`; a constant is a one-byte kind (`0` int, `1` float, `2` bool) and its
//! payload (`i64`, the `f64` bit pattern, or a `0`/`1` byte):
//!
//! ```text
//! 0x00 const       dst:u32  kind:u8  payload
//! 0x01 bin         op:u8    dst:u32  lhs:u32  rhs:u32
//! 0x02 un          op:u8    dst:u32  src:u32
//! 0x03 move        dst:u32  src:u32
//! 0x04 jump        target:u32
//! 0x05 jump_unless cond:u32 target:u32
//! 0x06 return      has_value:u8  [value:u32]
//! ```
//!
//! The encoding is lossless: every field of the program body round-trips exactly,
//! including `f64` bit patterns (NaN payloads survive). The layout is little-endian
//! on every target so an image produced on one host is byte-identical on another.

use alloc::vec::Vec;
use codegen_lang::{BinOp, Const, Op, Program, UnOp};

// Op tags. Stable: the byte written for a given op never changes, so encoded
// objects stay readable across releases.
const TAG_CONST: u8 = 0x00;
const TAG_BIN: u8 = 0x01;
const TAG_UN: u8 = 0x02;
const TAG_MOVE: u8 = 0x03;
const TAG_JUMP: u8 = 0x04;
const TAG_JUMP_UNLESS: u8 = 0x05;
const TAG_RETURN: u8 = 0x06;

// Constant kinds.
const KIND_INT: u8 = 0;
const KIND_FLOAT: u8 = 1;
const KIND_BOOL: u8 = 2;

/// Maps a binary operator to its stable byte code.
///
/// The match is exhaustive so a new IR operator is a compile error here rather than
/// a silently unencodable op.
fn bin_code(op: BinOp) -> u8 {
    match op {
        BinOp::Add => 0,
        BinOp::Sub => 1,
        BinOp::Mul => 2,
        BinOp::Div => 3,
        BinOp::Eq => 4,
        BinOp::Ne => 5,
        BinOp::Lt => 6,
        BinOp::Le => 7,
        BinOp::Gt => 8,
        BinOp::Ge => 9,
        BinOp::And => 10,
        BinOp::Or => 11,
    }
}

/// Maps a unary operator to its stable byte code.
fn un_code(op: UnOp) -> u8 {
    match op {
        UnOp::Neg => 0,
        UnOp::Not => 1,
    }
}

/// Encodes a lowered program into the section bytes described by the [module
/// docs](self).
///
/// The program's name is deliberately left out — the caller records it as the
/// object's entry symbol. The returned buffer is allocated once, sized from the
/// program up front, and filled without reallocating.
pub(crate) fn encode(program: &Program) -> Vec<u8> {
    let params = program.params();
    let ops = program.ops();

    // Header is 12 bytes plus four per parameter; a body op is between five and
    // fourteen bytes, so eight per op is a close lower-bound reservation.
    let mut out = Vec::with_capacity(12 + params.len() * 4 + ops.len() * 8);

    out.extend_from_slice(&program.register_count().to_le_bytes());
    // A parameter or op count that a validated function can reach always fits in a
    // u32: registers are u32-indexed and there is at most one op per IR instruction.
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for reg in params {
        out.extend_from_slice(&reg.0.to_le_bytes());
    }
    out.extend_from_slice(&(ops.len() as u32).to_le_bytes());

    for op in ops {
        encode_op(*op, &mut out);
    }

    out
}

/// Appends one op-record to `out`.
fn encode_op(op: Op, out: &mut Vec<u8>) {
    match op {
        Op::Const { dst, value } => {
            out.push(TAG_CONST);
            out.extend_from_slice(&dst.0.to_le_bytes());
            encode_const(value, out);
        }
        Op::Bin { op, dst, lhs, rhs } => {
            out.push(TAG_BIN);
            out.push(bin_code(op));
            out.extend_from_slice(&dst.0.to_le_bytes());
            out.extend_from_slice(&lhs.0.to_le_bytes());
            out.extend_from_slice(&rhs.0.to_le_bytes());
        }
        Op::Un { op, dst, src } => {
            out.push(TAG_UN);
            out.push(un_code(op));
            out.extend_from_slice(&dst.0.to_le_bytes());
            out.extend_from_slice(&src.0.to_le_bytes());
        }
        Op::Move { dst, src } => {
            out.push(TAG_MOVE);
            out.extend_from_slice(&dst.0.to_le_bytes());
            out.extend_from_slice(&src.0.to_le_bytes());
        }
        Op::Jump { target } => {
            out.push(TAG_JUMP);
            out.extend_from_slice(&target.0.to_le_bytes());
        }
        Op::JumpUnless { cond, target } => {
            out.push(TAG_JUMP_UNLESS);
            out.extend_from_slice(&cond.0.to_le_bytes());
            out.extend_from_slice(&target.0.to_le_bytes());
        }
        Op::Return { value } => {
            out.push(TAG_RETURN);
            match value {
                Some(reg) => {
                    out.push(1);
                    out.extend_from_slice(&reg.0.to_le_bytes());
                }
                None => out.push(0),
            }
        }
    }
}

/// Appends a constant's kind byte and payload to `out`.
fn encode_const(value: Const, out: &mut Vec<u8>) {
    match value {
        Const::Int(v) => {
            out.push(KIND_INT);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Const::Float(v) => {
            out.push(KIND_FLOAT);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        Const::Bool(v) => {
            out.push(KIND_BOOL);
            out.push(u8::from(v));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use alloc::vec;
    use codegen_lang::{Label, Reg, compile};
    use ir_lang::{Builder, Type};

    // A decoded program body, reconstructed from the bytes for round-trip checks.
    // Everything `encode` writes lands here; the name is compared separately since
    // it is not part of the encoding.
    #[derive(Debug, PartialEq)]
    struct Decoded {
        register_count: u32,
        params: Vec<Reg>,
        ops: Vec<Op>,
    }

    // A checked reader over the encoded bytes. Every read is bounds-checked, so
    // truncated or malformed input is a returned `None`, never a panic.
    struct Reader<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Reader { bytes, pos: 0 }
        }

        fn u8(&mut self) -> Option<u8> {
            let byte = *self.bytes.get(self.pos)?;
            self.pos += 1;
            Some(byte)
        }

        fn u32(&mut self) -> Option<u32> {
            let end = self.pos.checked_add(4)?;
            let slice = self.bytes.get(self.pos..end)?;
            self.pos = end;
            Some(u32::from_le_bytes(slice.try_into().ok()?))
        }

        fn i64(&mut self) -> Option<i64> {
            let end = self.pos.checked_add(8)?;
            let slice = self.bytes.get(self.pos..end)?;
            self.pos = end;
            Some(i64::from_le_bytes(slice.try_into().ok()?))
        }

        fn f64(&mut self) -> Option<f64> {
            let end = self.pos.checked_add(8)?;
            let slice = self.bytes.get(self.pos..end)?;
            self.pos = end;
            Some(f64::from_bits(u64::from_le_bytes(slice.try_into().ok()?)))
        }

        fn reg(&mut self) -> Option<Reg> {
            Some(Reg(self.u32()?))
        }

        fn label(&mut self) -> Option<Label> {
            Some(Label(self.u32()?))
        }
    }

    fn bin_from_code(code: u8) -> Option<BinOp> {
        Some(match code {
            0 => BinOp::Add,
            1 => BinOp::Sub,
            2 => BinOp::Mul,
            3 => BinOp::Div,
            4 => BinOp::Eq,
            5 => BinOp::Ne,
            6 => BinOp::Lt,
            7 => BinOp::Le,
            8 => BinOp::Gt,
            9 => BinOp::Ge,
            10 => BinOp::And,
            11 => BinOp::Or,
            _ => return None,
        })
    }

    fn un_from_code(code: u8) -> Option<UnOp> {
        Some(match code {
            0 => UnOp::Neg,
            1 => UnOp::Not,
            _ => return None,
        })
    }

    fn decode_const(r: &mut Reader<'_>) -> Option<Const> {
        Some(match r.u8()? {
            KIND_INT => Const::Int(r.i64()?),
            KIND_FLOAT => Const::Float(r.f64()?),
            KIND_BOOL => match r.u8()? {
                0 => Const::Bool(false),
                1 => Const::Bool(true),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn decode_op(r: &mut Reader<'_>) -> Option<Op> {
        Some(match r.u8()? {
            TAG_CONST => Op::Const {
                dst: r.reg()?,
                value: decode_const(r)?,
            },
            TAG_BIN => Op::Bin {
                op: bin_from_code(r.u8()?)?,
                dst: r.reg()?,
                lhs: r.reg()?,
                rhs: r.reg()?,
            },
            TAG_UN => Op::Un {
                op: un_from_code(r.u8()?)?,
                dst: r.reg()?,
                src: r.reg()?,
            },
            TAG_MOVE => Op::Move {
                dst: r.reg()?,
                src: r.reg()?,
            },
            TAG_JUMP => Op::Jump { target: r.label()? },
            TAG_JUMP_UNLESS => Op::JumpUnless {
                cond: r.reg()?,
                target: r.label()?,
            },
            TAG_RETURN => Op::Return {
                value: match r.u8()? {
                    0 => None,
                    1 => Some(r.reg()?),
                    _ => return None,
                },
            },
            _ => return None,
        })
    }

    fn decode(bytes: &[u8]) -> Option<Decoded> {
        let mut r = Reader::new(bytes);
        let register_count = r.u32()?;
        let param_count = r.u32()?;
        let mut params = Vec::with_capacity(param_count as usize);
        for _ in 0..param_count {
            params.push(r.reg()?);
        }
        let op_count = r.u32()?;
        let mut ops = Vec::with_capacity(op_count as usize);
        for _ in 0..op_count {
            ops.push(decode_op(&mut r)?);
        }
        // The whole buffer must be consumed; trailing bytes mean a malformed input.
        if r.pos != bytes.len() {
            return None;
        }
        Some(Decoded {
            register_count,
            params,
            ops,
        })
    }

    // Asserts that a compiled program survives an encode/decode round-trip intact.
    fn assert_round_trips(program: &Program) {
        let decoded = decode(&encode(program)).expect("encoding decodes cleanly");
        assert_eq!(decoded.register_count, program.register_count());
        assert_eq!(decoded.params, program.params());
        assert_eq!(decoded.ops, program.ops());
    }

    #[test]
    fn test_encode_straight_line_round_trips() {
        // fn f(x: int) -> int { x + x + x }
        let mut b = Builder::new("f", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let two_x = b.bin(BinOp::Add, x, x);
        let three_x = b.bin(BinOp::Add, two_x, x);
        b.ret(Some(three_x));
        assert_round_trips(&compile(&b.finish()).unwrap());
    }

    #[test]
    fn test_encode_unary_and_float_const_round_trips() {
        // fn f() -> float { -(3.5) }
        let mut b = Builder::new("f", &[], Type::Float);
        let c = b.fconst(3.5);
        let neg = b.un(UnOp::Neg, c);
        b.ret(Some(neg));
        assert_round_trips(&compile(&b.finish()).unwrap());
    }

    #[test]
    fn test_encode_bool_const_and_not_round_trips() {
        // fn f() -> bool { !true }
        let mut b = Builder::new("f", &[], Type::Bool);
        let t = b.bconst(true);
        let n = b.un(UnOp::Not, t);
        b.ret(Some(n));
        assert_round_trips(&compile(&b.finish()).unwrap());
    }

    #[test]
    fn test_encode_unit_return_round_trips() {
        // fn f() {}
        let mut b = Builder::new("f", &[], Type::Unit);
        b.ret(None);
        assert_round_trips(&compile(&b.finish()).unwrap());
    }

    #[test]
    fn test_encode_branch_round_trips() {
        // A two-way branch lowers to jump/jump_unless/move plus internal labels,
        // so this exercises the control-flow op records.
        // fn max(a: int, b: int) -> int { if a > b { a } else { b } }
        let mut b = Builder::new("max", &[Type::Int, Type::Int], Type::Int);
        let a = b.block_params(b.entry())[0];
        let bb = b.block_params(b.entry())[1];
        let then_blk = b.create_block(&[]);
        let else_blk = b.create_block(&[]);
        let cond = b.bin(BinOp::Gt, a, bb);
        b.branch(cond, then_blk, &[], else_blk, &[]);
        b.switch_to(then_blk);
        b.ret(Some(a));
        b.switch_to(else_blk);
        b.ret(Some(bb));
        assert_round_trips(&compile(&b.finish()).unwrap());
    }

    #[test]
    fn test_decode_rejects_truncated_input() {
        // A header claiming one op but no op bytes must not panic — it fails.
        let mut b = Builder::new("f", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        b.ret(Some(x));
        let mut bytes = encode(&compile(&b.finish()).unwrap());
        bytes.truncate(bytes.len() - 1);
        assert!(decode(&bytes).is_none());
    }

    #[test]
    fn test_decode_rejects_trailing_bytes() {
        let mut b = Builder::new("f", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        b.ret(Some(x));
        let mut bytes = encode(&compile(&b.finish()).unwrap());
        bytes.push(0xff);
        assert!(decode(&bytes).is_none());
    }

    #[test]
    fn test_decode_rejects_unknown_tag() {
        assert!(decode(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0x7f]).is_none());
    }

    #[test]
    fn test_encode_is_deterministic() {
        let build = || {
            let mut b = Builder::new("f", &[Type::Int], Type::Int);
            let x = b.block_params(b.entry())[0];
            let one = b.iconst(1);
            let y = b.bin(BinOp::Add, x, one);
            b.ret(Some(y));
            compile(&b.finish()).unwrap()
        };
        assert_eq!(encode(&build()), encode(&build()));
    }

    use proptest::prelude::*;

    proptest! {
        // Any straight-line integer function round-trips. The generator emits a mix
        // of constants and arithmetic over already-defined int values, so it covers
        // varied register counts, constant payloads, and Add/Sub/Mul/Div op records
        // while staying well-typed and valid.
        #[test]
        fn prop_straight_line_int_round_trips(
            steps in prop::collection::vec(
                prop_oneof![
                    any::<i64>().prop_map(Step::Const),
                    (0usize..4, 0usize..4, 0u8..4).prop_map(|(l, r, op)| Step::Bin(l, r, op)),
                ],
                1..40,
            )
        ) {
            let program = compile(&build_int_program(&steps)).unwrap();
            let decoded = decode(&encode(&program)).expect("decodes");
            prop_assert_eq!(decoded.register_count, program.register_count());
            prop_assert_eq!(&decoded.params, program.params());
            prop_assert_eq!(&decoded.ops, program.ops());
        }
    }

    // A generated build step over integer values.
    #[derive(Clone, Debug)]
    enum Step {
        Const(i64),
        // A binary op combining two earlier values (indices are taken modulo the
        // number of values defined so far) using one of Add/Sub/Mul/Div.
        Bin(usize, usize, u8),
    }

    fn build_int_program(steps: &[Step]) -> ir_lang::Function {
        let mut b = Builder::new("gen", &[Type::Int, Type::Int], Type::Int);
        let mut values = vec![b.block_params(b.entry())[0], b.block_params(b.entry())[1]];
        for step in steps {
            let value = match *step {
                Step::Const(v) => b.iconst(v),
                Step::Bin(l, r, op) => {
                    let lhs = values[l % values.len()];
                    let rhs = values[r % values.len()];
                    let op = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div][op as usize % 4];
                    b.bin(op, lhs, rhs)
                }
            };
            values.push(value);
        }
        let last = *values.last().expect("at least the two parameters exist");
        b.ret(Some(last));
        b.finish()
    }
}
