//! The object-code format: how a lowered [`Program`] becomes the bytes of a
//! linkable section.
//!
//! A [`codegen_lang::Program`] is a name, a list of parameter registers, a register
//! count, a label table, and a stream of [`Op`]s. [`encode`] flattens everything
//! except the name into a compact little-endian byte string; the name lives in the
//! object's symbol table, not its bytes, because that is what the linker resolves an
//! address for.
//!
//! ## Layout (format version 1)
//!
//! All integers are little-endian. Each function's bytes are one self-delimiting
//! record: a header, the label table, then one record per op.
//!
//! ```text
//! header:
//!   magic          : 4 bytes  "AOTB"
//!   version        : u32      1
//!   register_count : u32
//!   param_count    : u32
//!   params         : param_count × u32   (each parameter's register number)
//!   label_count    : u32
//!   labels         : label_count × u32   (label i resolves to op index labels[i])
//!   op_count       : u32
//! body:
//!   op_count × op-record
//! ```
//!
//! The magic and version let a reader refuse bytes it does not understand instead
//! of misreading them. `1.0.0` wrote no header and no label table (an unversioned
//! format, retroactively version 0), so jump targets in its output could not be
//! resolved; a version-1 reader rejects those bytes at the magic.
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
//! 0x04 jump        target:u32                 (a label id; see the label table)
//! 0x05 jump_unless cond:u32 target:u32        (a label id)
//! 0x06 return      has_value:u8  [value:u32]
//! ```
//!
//! The encoding is lossless for the program body: register count, parameters, the
//! complete label table, and every op round-trip exactly, including `f64` bit
//! patterns (NaN payloads survive). The tests check this with a decoder over the
//! whole program. The layout is little-endian on every target, so an image produced
//! on one host is byte-identical on another.

use alloc::vec::Vec;
use codegen_lang::{BinOp, Const, Label, Op, Program, UnOp};

/// The four bytes every encoded function starts with.
const MAGIC: [u8; 4] = *b"AOTB";

/// The format version written after the magic. Bumped whenever the layout changes,
/// so a reader can refuse a format it does not know.
const FORMAT_VERSION: u32 = 1;

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

/// The fixed part of the header: magic, version, register count, parameter count,
/// label count, and op count, four bytes each.
const HEADER_FIXED: usize = 24;

/// Maps a binary operator to its stable byte code.
///
/// The match is exhaustive so a new IR operator is a compile error here rather than
/// a silently unencodable op.
#[inline]
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
#[inline]
fn un_code(op: UnOp) -> u8 {
    match op {
        UnOp::Neg => 0,
        UnOp::Not => 1,
    }
}

/// The exact number of bytes one op-record occupies, matching what [`encode_op`]
/// writes. Kept in lockstep with the encoder so [`encoded_len`] is exact.
#[inline]
fn op_size(op: &Op) -> usize {
    match op {
        // tag + dst + (kind + payload)
        Op::Const { value, .. } => 5 + const_payload_size(value),
        // tag + op + dst + lhs + rhs
        Op::Bin { .. } => 14,
        // tag + op + dst + src
        Op::Un { .. } => 10,
        // tag + dst + src
        Op::Move { .. } => 9,
        // tag + target
        Op::Jump { .. } => 5,
        // tag + cond + target
        Op::JumpUnless { .. } => 9,
        // tag + has_value + (optional value)
        Op::Return { value } => 2 + if value.is_some() { 4 } else { 0 },
    }
}

/// The bytes a constant occupies after its op tag and destination register: one
/// kind byte plus the payload (eight for int/float, one for bool).
#[inline]
fn const_payload_size(value: &Const) -> usize {
    1 + match value {
        Const::Int(_) | Const::Float(_) => 8,
        Const::Bool(_) => 1,
    }
}

/// The number of entries in `program`'s label table.
///
/// codegen-lang 1.0 exposes the table only through [`Program::label_offset`], but
/// label ids are dense from `0` (one per block, then the internal labels the
/// lowering adds), so the count is the first id that does not resolve. The walk is
/// linear in the label count and allocates nothing. A table of more than `u32::MAX`
/// entries (16 GiB of offsets in memory) is not representable in the format; the
/// count stops at `u32::MAX`.
#[inline]
fn label_count(program: &Program) -> u32 {
    let mut count: u32 = 0;
    while program.label_offset(Label(count)).is_some() {
        match count.checked_add(1) {
            Some(next) => count = next,
            None => break,
        }
    }
    count
}

/// Narrows a length or op index to the format's `u32` field.
///
/// Every value written this way is bounded by `u32` upstream: register numbers and
/// label ids are `u32` handles, and codegen-lang stores label offsets as `u32`, so
/// its programs never hold more ops than a `u32` index reaches. For a program that
/// somehow exceeded it the field saturates at `u32::MAX` instead of wrapping, which
/// makes the record fail to decode (a count that disagrees with the bytes) rather
/// than silently alias a smaller one.
#[inline]
fn narrow(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The exact number of bytes [`encode`] will write for `program`, whose label table
/// has `labels` entries, so the output buffer is allocated once and never grows.
///
/// The sum uses saturating arithmetic: the byte count of a real program is far
/// inside `usize`, and for a pathological one the reservation caps out rather than
/// wrapping to a too-small capacity. A wrong hint would only cost a reallocation,
/// never correctness, but computing it exactly means the common path allocates once.
#[inline]
fn encoded_len(program: &Program, labels: u32) -> usize {
    let params_bytes = program.params().len().saturating_mul(4);
    let label_bytes = (labels as usize).saturating_mul(4);
    let mut total = HEADER_FIXED
        .saturating_add(params_bytes)
        .saturating_add(label_bytes);
    for op in program.ops() {
        total = total.saturating_add(op_size(op));
    }
    total
}

/// Encodes a lowered program into the section bytes described by the [module
/// docs](self).
///
/// The program's name is deliberately left out — the caller records it as the
/// object's entry symbol. The buffer is sized exactly from the program up front, so
/// it is allocated once and filled without reallocating.
#[inline]
pub(crate) fn encode(program: &Program) -> Vec<u8> {
    let params = program.params();
    let ops = program.ops();
    let labels = label_count(program);

    let mut out = Vec::with_capacity(encoded_len(program, labels));

    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&program.register_count().to_le_bytes());
    out.extend_from_slice(&narrow(params.len()).to_le_bytes());
    for reg in params {
        out.extend_from_slice(&reg.0.to_le_bytes());
    }
    out.extend_from_slice(&labels.to_le_bytes());
    for id in 0..labels {
        // Every id below `labels` resolves (that is how the count was found); the
        // fallback only keeps this loop free of a panic path.
        let offset = program.label_offset(Label(id)).map_or(u32::MAX, narrow);
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&narrow(ops.len()).to_le_bytes());

    for op in ops {
        encode_op(*op, &mut out);
    }

    out
}

/// Appends one op-record to `out`.
#[inline]
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
#[inline]
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
    use crate::test_decode::{self, Body, DecodeError, body_of, decode_exact};
    use alloc::vec;
    use codegen_lang::{Reg, compile};
    use ir_lang::{Builder, Type};

    // Asserts that a compiled program survives an encode/decode round-trip with every
    // field intact (register count, parameters, the full label table, every op), and
    // that the buffer was sized exactly.
    fn assert_round_trips(program: &Program) -> Body {
        let bytes = encode(program);
        assert_eq!(
            bytes.len(),
            encoded_len(program, label_count(program)),
            "size prediction is exact"
        );
        assert_eq!(bytes.len(), bytes.capacity(), "no reallocation occurred");
        let decoded = decode_exact(&bytes).expect("encoding decodes cleanly");
        assert_eq!(decoded, body_of(program));
        decoded
    }

    // fn max(a: int, b: int) -> int { if a > b { a } else { b } }
    fn max_fn() -> ir_lang::Function {
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
        b.finish()
    }

    // fn swap_loop(a: int, b: int, n: int) -> int: `n` times, `(a, b) = (b, a)`; returns a.
    // The back edge permutes the header's own parameters.
    fn swap_loop_fn() -> ir_lang::Function {
        let mut b = Builder::new("swap_loop", &[Type::Int, Type::Int, Type::Int], Type::Int);
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

    #[test]
    fn test_header_starts_with_magic_and_version() {
        let mut b = Builder::new("f", &[], Type::Unit);
        b.ret(None);
        let bytes = encode(&compile(&b.finish()).unwrap());
        assert_eq!(&bytes[..4], b"AOTB");
        assert_eq!(bytes[4..8], 1u32.to_le_bytes());
        assert_eq!(MAGIC, test_decode::MAGIC);
        assert_eq!(FORMAT_VERSION, test_decode::VERSION);
    }

    #[test]
    fn test_encode_straight_line_round_trips() {
        // fn f(x: int) -> int { x + x + x }
        let mut b = Builder::new("f", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let two_x = b.bin(BinOp::Add, x, x);
        let three_x = b.bin(BinOp::Add, two_x, x);
        b.ret(Some(three_x));
        let body = assert_round_trips(&compile(&b.finish()).unwrap());
        // One block, so one label, at op 0.
        assert_eq!(body.labels, vec![0]);
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
    fn test_encode_nan_payload_round_trips() {
        let mut b = Builder::new("f", &[], Type::Float);
        let c = b.fconst(f64::from_bits(0x7ff8_dead_beef_0001));
        b.ret(Some(c));
        let body = assert_round_trips(&compile(&b.finish()).unwrap());
        assert!(body.ops.iter().any(|op| matches!(
            op,
            Op::Const { value: Const::Float(v), .. } if v.to_bits() == 0x7ff8_dead_beef_0001
        )));
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
    fn test_encode_branch_round_trips_with_its_label_table() {
        // A two-way branch lowers to jump_unless/jump plus internal labels; the
        // label table is what resolves those jump targets (ISSUES H02).
        let program = compile(&max_fn()).unwrap();
        let body = assert_round_trips(&program);
        assert!(body.labels.len() >= 3, "entry, then, else");
        let mut jumps = 0;
        for op in &body.ops {
            let target = match *op {
                Op::Jump { target } | Op::JumpUnless { target, .. } => target,
                _ => continue,
            };
            jumps += 1;
            let at = body
                .label_offset(target)
                .expect("every target is in the table");
            assert!(at < body.ops.len());
            assert_eq!(Some(at), program.label_offset(target));
        }
        assert!(jumps > 0);
    }

    #[test]
    fn test_encode_loop_with_permuting_back_edge_round_trips() {
        assert_round_trips(&compile(&swap_loop_fn()).unwrap());
    }

    #[test]
    fn test_decode_rejects_every_truncation() {
        let bytes = encode(&compile(&max_fn()).unwrap());
        for len in 0..bytes.len() {
            assert!(
                decode_exact(&bytes[..len]).is_err(),
                "prefix of {len} bytes"
            );
        }
    }

    #[test]
    fn test_decode_rejects_trailing_bytes() {
        let mut bytes = encode(&compile(&max_fn()).unwrap());
        bytes.push(0xff);
        assert_eq!(decode_exact(&bytes), Err(DecodeError::TrailingBytes));
    }

    #[test]
    fn test_decode_rejects_unknown_tag() {
        // A valid empty header (no registers, params, labels) claiming one op whose
        // tag is unknown.
        let mut bytes = vec![];
        bytes.extend_from_slice(b"AOTB");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 12]); // registers, params, labels
        bytes.extend_from_slice(&1u32.to_le_bytes()); // op count
        bytes.extend_from_slice(&[0x7f, 0]);
        assert_eq!(
            decode_exact(&bytes),
            Err(DecodeError::BadValue { byte: 0x7f, at: 24 })
        );
    }

    #[test]
    fn test_decode_refuses_other_versions_and_bad_magic() {
        let good = encode(&compile(&max_fn()).unwrap());
        for version in [0u32, 2, u32::MAX] {
            let mut bytes = good.clone();
            bytes[4..8].copy_from_slice(&version.to_le_bytes());
            assert_eq!(
                decode_exact(&bytes),
                Err(DecodeError::UnsupportedVersion(version))
            );
        }
        let mut bytes = good;
        bytes[0] = b'X';
        assert_eq!(decode_exact(&bytes), Err(DecodeError::BadMagic));
    }

    // The 1.0.0 layout, reproduced here only to show that its output is refused:
    // register count, params, op count, ops, with no magic and no label table.
    fn encode_v0(program: &Program) -> Vec<u8> {
        let mut out = vec![];
        out.extend_from_slice(&program.register_count().to_le_bytes());
        out.extend_from_slice(&narrow(program.params().len()).to_le_bytes());
        for reg in program.params() {
            out.extend_from_slice(&reg.0.to_le_bytes());
        }
        out.extend_from_slice(&narrow(program.ops().len()).to_le_bytes());
        for op in program.ops() {
            encode_op(*op, &mut out);
        }
        out
    }

    #[test]
    fn test_decode_refuses_the_unversioned_1_0_0_layout() {
        for func in [max_fn(), swap_loop_fn()] {
            let old = encode_v0(&compile(&func).unwrap());
            assert_eq!(decode_exact(&old), Err(DecodeError::BadMagic));
        }
    }

    #[test]
    fn test_decode_refuses_a_hostile_count_without_allocating_it() {
        let mut bytes = vec![];
        bytes.extend_from_slice(b"AOTB");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // registers
        bytes.extend_from_slice(&0u32.to_le_bytes()); // params
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // labels: 4 billion
        assert_eq!(decode_exact(&bytes), Err(DecodeError::CountTooLarge));
    }

    #[test]
    fn test_label_count_matches_the_decoded_table() {
        for func in [max_fn(), swap_loop_fn()] {
            let program = compile(&func).unwrap();
            let n = label_count(&program);
            assert_eq!(n as usize, body_of(&program).labels.len());
            assert!(program.label_offset(Label(n)).is_none());
        }
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

    #[test]
    fn test_parameters_round_trip_in_order() {
        let mut b = Builder::new("f", &[Type::Int, Type::Float, Type::Bool], Type::Unit);
        b.ret(None);
        let body = assert_round_trips(&compile(&b.finish()).unwrap());
        assert_eq!(body.params, vec![Reg(0), Reg(1), Reg(2)]);
    }

    use proptest::prelude::*;

    proptest! {
        // Any straight-line integer function round-trips. The generator emits a mix
        // of constants and arithmetic over already-defined int values, so it covers
        // varied register counts, constant payloads, and Add/Sub/Mul/Div op records
        // while staying well-typed and valid. (Control flow is covered by the CFG
        // properties in `tests/format.rs`.)
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
            let bytes = encode(&program);
            // The size prediction is exact, so the buffer is allocated once.
            prop_assert_eq!(bytes.len(), encoded_len(&program, label_count(&program)));
            prop_assert_eq!(bytes.len(), bytes.capacity());
            prop_assert_eq!(decode_exact(&bytes).expect("decodes"), body_of(&program));
        }

        // Arbitrary bytes never panic the reader (they almost always fail to decode).
        #[test]
        fn prop_decoder_is_total_on_arbitrary_bytes(
            tail in prop::collection::vec(any::<u8>(), 0..64),
            with_header in any::<bool>(),
        ) {
            let mut bytes = vec![];
            if with_header {
                bytes.extend_from_slice(b"AOTB");
                bytes.extend_from_slice(&1u32.to_le_bytes());
            }
            bytes.extend_from_slice(&tail);
            let _ = decode_exact(&bytes);
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
