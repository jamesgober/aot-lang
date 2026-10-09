//! A reader for aot-lang's object-code format (version 1). **Tests only.**
//!
//! It is compiled into the crate's unit tests (`src/lib.rs` includes this file under
//! `#[cfg(test)]`) and into the integration tests (`tests/support/mod.rs`); it is not
//! part of the published API. It states the format independently of the encoder:
//! every tag, kind, and field width here is written from the documented layout, not
//! imported from `src/encode.rs`, so an encoder that drifts from the documentation
//! fails the round-trip tests.
//!
//! Every read is bounds-checked and every count is checked against the bytes left,
//! so hostile or truncated input returns an error and never panics or over-allocates.

#![allow(
    dead_code,
    reason = "each test crate that includes this file uses a different subset"
)]

use alloc::vec::Vec;
use codegen_lang::{BinOp, Const, Label, Op, Program, Reg, UnOp};

/// The magic every version-1 record starts with.
pub const MAGIC: [u8; 4] = *b"AOTB";
/// The only format version this reader understands.
pub const VERSION: u32 = 1;

/// One function's program body as the bytes describe it: every field of a
/// [`Program`] except its name, which the image's symbol table carries.
///
/// Equality is exact: float constants compare by bit pattern, so a NaN payload or
/// the sign of a zero that did not survive encoding is a mismatch (`f64 ==` would
/// call every NaN unequal and `-0.0` equal to `0.0`).
#[derive(Clone, Debug)]
pub struct Body {
    /// The number of registers (`Program::register_count`).
    pub register_count: u32,
    /// The parameter registers, in order (`Program::params`).
    pub params: Vec<Reg>,
    /// The label table: entry `i` is the op index label `i` resolves to
    /// (`Program::label_offset`).
    pub labels: Vec<u32>,
    /// The op stream (`Program::ops`).
    pub ops: Vec<Op>,
}

impl PartialEq for Body {
    fn eq(&self, other: &Self) -> bool {
        self.register_count == other.register_count
            && self.params == other.params
            && self.labels == other.labels
            && self.ops.len() == other.ops.len()
            && self.ops.iter().zip(&other.ops).all(|(a, b)| same_op(a, b))
    }
}

/// Op equality with float constants compared bit for bit.
fn same_op(a: &Op, b: &Op) -> bool {
    match (a, b) {
        (
            Op::Const {
                dst: da,
                value: Const::Float(x),
            },
            Op::Const {
                dst: db,
                value: Const::Float(y),
            },
        ) => da == db && x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

impl Body {
    /// The op index a label resolves to, as `Program::label_offset` reports it.
    #[must_use]
    pub fn label_offset(&self, label: Label) -> Option<usize> {
        self.labels.get(label.0 as usize).map(|&o| o as usize)
    }
}

/// Every field of `program` except the name, read through codegen-lang's public
/// accessors. `Program` has exactly five fields (name, params, registers, ops,
/// labels) and no public constructor, so this projection plus the name is how a
/// decoded record is compared with "the full `Program`".
#[must_use]
pub fn body_of(program: &Program) -> Body {
    let mut labels = Vec::new();
    let mut id: u32 = 0;
    while let Some(offset) = program.label_offset(Label(id)) {
        labels.push(u32::try_from(offset).unwrap_or(u32::MAX));
        match id.checked_add(1) {
            Some(next) => id = next,
            None => break,
        }
    }
    Body {
        register_count: program.register_count(),
        params: program.params().to_vec(),
        labels,
        ops: program.ops().to_vec(),
    }
}

/// Why bytes could not be decoded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeError {
    /// The record does not start with [`MAGIC`] (for example, `1.0.0`'s headerless
    /// output, or bytes that are not aot-lang object code).
    BadMagic,
    /// The magic matched but the version is not [`VERSION`].
    UnsupportedVersion(u32),
    /// The bytes end before the record does.
    Truncated,
    /// An unknown op tag, operator code, constant kind, bool byte, or return flag.
    BadValue {
        /// The offending byte.
        byte: u8,
        /// Its position in the input.
        at: usize,
    },
    /// A count claims more entries than the remaining bytes could hold.
    CountTooLarge,
    /// [`decode_exact`] found bytes after the record.
    TrailingBytes,
}

/// A checked little-endian cursor.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        self.take(1)?.first().copied().ok_or(DecodeError::Truncated)
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let mut b = [0u8; 4];
        b.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(b))
    }

    fn i64(&mut self) -> Result<i64, DecodeError> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(i64::from_le_bytes(b))
    }

    fn reg(&mut self) -> Result<Reg, DecodeError> {
        Ok(Reg(self.u32()?))
    }

    fn label(&mut self) -> Result<Label, DecodeError> {
        Ok(Label(self.u32()?))
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// A `u32` count of entries at least `min_size` bytes each, refused if the
    /// remaining input could not hold that many: a hostile count cannot make the
    /// reader reserve memory the bytes do not back.
    fn count(&mut self, min_size: usize) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > self.remaining() / min_size {
            return Err(DecodeError::CountTooLarge);
        }
        Ok(n)
    }

    fn bad(&self, byte: u8) -> DecodeError {
        DecodeError::BadValue {
            byte,
            at: self.pos.saturating_sub(1),
        }
    }
}

fn bin_op(r: &Reader<'_>, code: u8) -> Result<BinOp, DecodeError> {
    Ok(match code {
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
        other => return Err(r.bad(other)),
    })
}

fn un_op(r: &Reader<'_>, code: u8) -> Result<UnOp, DecodeError> {
    Ok(match code {
        0 => UnOp::Neg,
        1 => UnOp::Not,
        other => return Err(r.bad(other)),
    })
}

fn constant(r: &mut Reader<'_>) -> Result<Const, DecodeError> {
    Ok(match r.u8()? {
        0 => Const::Int(r.i64()?),
        1 => Const::Float(f64::from_bits(r.u64()?)),
        2 => match r.u8()? {
            0 => Const::Bool(false),
            1 => Const::Bool(true),
            other => return Err(r.bad(other)),
        },
        other => return Err(r.bad(other)),
    })
}

fn op(r: &mut Reader<'_>) -> Result<Op, DecodeError> {
    Ok(match r.u8()? {
        0x00 => Op::Const {
            dst: r.reg()?,
            value: constant(r)?,
        },
        0x01 => {
            let code = r.u8()?;
            Op::Bin {
                op: bin_op(r, code)?,
                dst: r.reg()?,
                lhs: r.reg()?,
                rhs: r.reg()?,
            }
        }
        0x02 => {
            let code = r.u8()?;
            Op::Un {
                op: un_op(r, code)?,
                dst: r.reg()?,
                src: r.reg()?,
            }
        }
        0x03 => Op::Move {
            dst: r.reg()?,
            src: r.reg()?,
        },
        0x04 => Op::Jump { target: r.label()? },
        0x05 => Op::JumpUnless {
            cond: r.reg()?,
            target: r.label()?,
        },
        0x06 => Op::Return {
            value: match r.u8()? {
                0 => None,
                1 => Some(r.reg()?),
                other => return Err(r.bad(other)),
            },
        },
        other => return Err(r.bad(other)),
    })
}

/// Decodes one record from the start of `bytes`, returning the body and the number
/// of bytes it occupied. Bytes after the record are left alone, which is how a
/// function is read out of an image's `.text` section at its symbol's offset.
pub fn decode_prefix(bytes: &[u8]) -> Result<(Body, usize), DecodeError> {
    let mut r = Reader { bytes, pos: 0 };
    if r.take(4).map_err(|_| DecodeError::BadMagic)? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let register_count = r.u32()?;

    let param_count = r.count(4)?;
    let mut params = Vec::with_capacity(param_count);
    for _ in 0..param_count {
        params.push(r.reg()?);
    }

    let label_count = r.count(4)?;
    let mut labels = Vec::with_capacity(label_count);
    for _ in 0..label_count {
        labels.push(r.u32()?);
    }

    // The smallest op record (a unit return) is two bytes.
    let op_count = r.count(2)?;
    let mut ops = Vec::with_capacity(op_count);
    for _ in 0..op_count {
        ops.push(op(&mut r)?);
    }

    Ok((
        Body {
            register_count,
            params,
            labels,
            ops,
        },
        r.pos,
    ))
}

/// Decodes `bytes` as exactly one record; trailing bytes are an error.
pub fn decode_exact(bytes: &[u8]) -> Result<Body, DecodeError> {
    let (body, used) = decode_prefix(bytes)?;
    if used != bytes.len() {
        return Err(DecodeError::TrailingBytes);
    }
    Ok(body)
}
