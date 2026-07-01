//! The failure type for an ahead-of-time compile.

use codegen_lang::CodegenError;
use core::fmt;
use linker_lang::LinkError;

/// The reason an ahead-of-time compile could not be completed.
///
/// A compile runs two stages — lower each function to object code, then link the
/// objects into an image — and either stage can fail. The two variants keep the
/// origin of a failure visible instead of flattening both into one opaque message,
/// and each carries the underlying error so the exact cause can be inspected or
/// reported without guesswork.
///
/// `AotError` implements [`Display`](fmt::Display) and
/// [`core::error::Error`], with [`source`](core::error::Error::source) set to the
/// wrapped error, and converts from both underlying errors so `?` propagates them
/// directly. The enum is `#[non_exhaustive]`: a later stage that reports a new kind
/// of failure is an additive change, so a `match` on it must keep a wildcard arm.
///
/// # Examples
///
/// A malformed function is rejected during lowering, before any linking happens:
///
/// ```
/// use aot_lang::{compile, AotError};
/// use ir_lang::{Builder, Type};
///
/// // Declares an int return but never returns a value.
/// let func = Builder::new("bad", &[], Type::Int).finish();
///
/// assert!(matches!(compile(&func), Err(AotError::Codegen(_))));
/// ```
///
/// Inspect the source through the [`Error`](core::error::Error) trait:
///
/// ```
/// use aot_lang::compile;
/// use core::error::Error;
/// use ir_lang::{Builder, Type};
///
/// let func = Builder::new("bad", &[], Type::Int).finish();
/// let err = compile(&func).unwrap_err();
/// assert!(err.source().is_some());
/// ```
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AotError {
    /// A function could not be lowered to object code. The wrapped
    /// [`CodegenError`] names the offending block or value; the usual cause is IR
    /// that does not pass `Function::validate`.
    Codegen(CodegenError),
    /// The objects could not be linked into an image. The wrapped [`LinkError`]
    /// points at the symbol, section, or relocation involved — most often a
    /// duplicate function name or an undefined entry point.
    Link(LinkError),
}

impl fmt::Display for AotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AotError::Codegen(err) => write!(f, "code generation failed: {err}"),
            AotError::Link(err) => write!(f, "linking failed: {err}"),
        }
    }
}

impl core::error::Error for AotError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            AotError::Codegen(err) => Some(err),
            AotError::Link(err) => Some(err),
        }
    }
}

impl From<CodegenError> for AotError {
    fn from(err: CodegenError) -> Self {
        AotError::Codegen(err)
    }
}

impl From<LinkError> for AotError {
    fn from(err: LinkError) -> Self {
        AotError::Link(err)
    }
}
