//! The compiler that ties the pipeline together: lower each function to object
//! code, then link the objects into one image.

use crate::encode::encode;
use crate::error::AotError;
use alloc::string::String;
use alloc::vec::Vec;
use codegen_lang::{Program, compile as lower};
use ir_lang::Function;
use linker_lang::{Image, Linker, Object};

/// The section every compiled function's code is placed in.
const TEXT: &str = ".text";

/// Builds the linker object for one lowered program: its encoded code in the
/// [`TEXT`] section, with a symbol at the start named for the function.
fn object_from_program(program: &Program) -> Object {
    let mut object = Object::new(program.name());
    object.section(TEXT, encode(program));
    object.define(program.name(), TEXT, 0);
    object
}

/// Ahead-of-time compile a single function into a linked [`Image`], with the
/// function itself as the entry point.
///
/// This is the shortcut for the common case of compiling one function. It lowers
/// `func` to object code, links it at base address `0`, and sets the image's entry
/// to the function. Reach for [`Compiler`] when you have several functions to place
/// in one image, or need to choose the base address or entry point.
///
/// # Parameters
///
/// - `func` — the function to compile, in SSA form as produced by `ir_lang::Builder`.
///   It is validated during lowering.
///
/// # Errors
///
/// Returns [`AotError::Codegen`] if `func` does not pass `Function::validate`. The
/// link step cannot fail for a single well-formed function, so no [`AotError::Link`]
/// arises here.
///
/// # Examples
///
/// Compile a function and read back its code and entry point:
///
/// ```
/// use aot_lang::compile;
/// use ir_lang::{Builder, BinOp, Type};
///
/// // fn square(x: int) -> int { x * x }
/// let mut b = Builder::new("square", &[Type::Int], Type::Int);
/// let x = b.block_params(b.entry())[0];
/// let sq = b.bin(BinOp::Mul, x, x);
/// b.ret(Some(sq));
///
/// let image = compile(&b.finish()).expect("square is well-formed");
///
/// // The entry point is `square`, laid out at the default base address of 0.
/// assert_eq!(image.entry(), Some(0));
/// assert_eq!(image.symbol("square"), Some(0));
/// // Its object code is the `.text` section's bytes.
/// assert!(!image.section(".text").unwrap().data().is_empty());
/// ```
///
/// A function that does not validate is rejected:
///
/// ```
/// use aot_lang::{compile, AotError};
/// use ir_lang::{Builder, Type};
///
/// // Promises an int return but never returns one.
/// let func = Builder::new("bad", &[], Type::Int).finish();
/// assert!(matches!(compile(&func), Err(AotError::Codegen(_))));
/// ```
pub fn compile(func: &Function) -> Result<Image, AotError> {
    let program = lower(func)?;
    let image = Linker::new()
        .entry(program.name())
        .link(&[object_from_program(&program)])?;
    Ok(image)
}

/// Accumulates functions and links them into one image.
///
/// A `Compiler` compiles each function you [`add`](Compiler::add) to object code
/// and holds it; [`link`](Compiler::link) places them all in a single [`Image`].
/// This is the multi-function counterpart to the free [`compile`] function: use it
/// to build an image out of several functions, to choose the
/// [`base_address`](Compiler::base_address) the image is laid out at, or to name the
/// [`entry`](Compiler::entry) point.
///
/// Functions are laid out in the order they are added. Each defines a symbol under
/// its own name, so two functions with the same name collide — the link then fails
/// with [`AotError::Link`].
///
/// Implements [`Default`], equivalent to [`Compiler::new`]: base address `0`, no
/// entry point.
///
/// # Examples
///
/// Place two functions in one image, based at a load address, with a chosen entry
/// point:
///
/// ```
/// use aot_lang::Compiler;
/// use ir_lang::{Builder, BinOp, Type};
///
/// // fn add(a: int, b: int) -> int { a + b }
/// let mut add = Builder::new("add", &[Type::Int, Type::Int], Type::Int);
/// let a = add.block_params(add.entry())[0];
/// let b = add.block_params(add.entry())[1];
/// let sum = add.bin(BinOp::Add, a, b);
/// add.ret(Some(sum));
///
/// // fn one() -> int { 1 }
/// let mut one = Builder::new("one", &[], Type::Int);
/// let c = one.iconst(1);
/// one.ret(Some(c));
///
/// let mut compiler = Compiler::new();
/// compiler.base_address(0x40_0000).entry("add");
/// compiler.add(&add.finish()).unwrap();
/// compiler.add(&one.finish()).unwrap();
/// let image = compiler.link().unwrap();
///
/// // `add` is the entry, placed first at the base address; `one` follows it.
/// assert_eq!(image.entry(), Some(0x40_0000));
/// assert_eq!(image.symbol("add"), Some(0x40_0000));
/// assert!(image.symbol("one").unwrap() > 0x40_0000);
/// ```
#[derive(Default)]
pub struct Compiler {
    objects: Vec<Object>,
    base_address: u64,
    entry: Option<String>,
}

impl Compiler {
    /// Creates an empty compiler with the default configuration: base address `0`
    /// and no entry point. Same as [`Compiler::default`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the address the image is laid out from. The first function is placed
    /// here and the rest follow; every resolved symbol address is relative to it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aot_lang::Compiler;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut f = Builder::new("f", &[], Type::Unit);
    /// f.ret(None);
    ///
    /// let mut compiler = Compiler::new();
    /// compiler.base_address(0x1000);
    /// compiler.add(&f.finish()).unwrap();
    /// assert_eq!(compiler.link().unwrap().symbol("f"), Some(0x1000));
    /// ```
    pub fn base_address(&mut self, addr: u64) -> &mut Self {
        self.base_address = addr;
        self
    }

    /// Sets the symbol whose address becomes the image's entry point. The symbol
    /// must name a function that is added before [`link`](Compiler::link), or the
    /// link fails with [`AotError::Link`].
    ///
    /// # Examples
    ///
    /// ```
    /// use aot_lang::Compiler;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut main = Builder::new("main", &[], Type::Unit);
    /// main.ret(None);
    ///
    /// let mut compiler = Compiler::new();
    /// compiler.entry("main");
    /// compiler.add(&main.finish()).unwrap();
    /// assert_eq!(compiler.link().unwrap().entry(), Some(0));
    /// ```
    pub fn entry(&mut self, symbol: impl Into<String>) -> &mut Self {
        self.entry = Some(symbol.into());
        self
    }

    /// Compiles `func` to object code and appends it to the image being built.
    ///
    /// The function is lowered immediately, so a malformed one is reported here
    /// rather than deferred to [`link`](Compiler::link). Functions keep the order in
    /// which they are added, which is the order they are placed in the image.
    ///
    /// # Errors
    ///
    /// Returns [`AotError::Codegen`] if `func` does not pass `Function::validate`.
    ///
    /// # Examples
    ///
    /// ```
    /// use aot_lang::Compiler;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut compiler = Compiler::new();
    /// for name in ["a", "b", "c"] {
    ///     let mut f = Builder::new(name, &[], Type::Unit);
    ///     f.ret(None);
    ///     compiler.add(&f.finish()).unwrap();
    /// }
    /// assert_eq!(compiler.link().unwrap().symbols().count(), 3);
    /// ```
    pub fn add(&mut self, func: &Function) -> Result<&mut Self, AotError> {
        let program = lower(func)?;
        self.objects.push(object_from_program(&program));
        Ok(self)
    }

    /// Links every added function into one [`Image`].
    ///
    /// The functions are placed end to end from the [base
    /// address](Compiler::base_address), each defining a symbol under its name, and
    /// the [entry point](Compiler::entry) — if one was set — resolves to its address.
    ///
    /// # Errors
    ///
    /// Returns [`AotError::Link`] if two functions share a name
    /// (`LinkError::DuplicateSymbol`), or if the configured entry point was never
    /// added (`LinkError::UndefinedEntry`).
    ///
    /// # Examples
    ///
    /// ```
    /// use aot_lang::Compiler;
    /// use ir_lang::{Builder, Type};
    ///
    /// let mut f = Builder::new("f", &[], Type::Unit);
    /// f.ret(None);
    ///
    /// let mut compiler = Compiler::new();
    /// compiler.add(&f.finish()).unwrap();
    /// let image = compiler.link().unwrap();
    /// assert_eq!(image.len(), 1); // one `.text` section
    /// ```
    pub fn link(&self) -> Result<Image, AotError> {
        let mut linker = Linker::new().base_address(self.base_address);
        if let Some(entry) = &self.entry {
            linker = linker.entry(entry);
        }
        linker.link(&self.objects).map_err(AotError::from)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::AotError;
    use ir_lang::{BinOp, Builder, Type};
    use linker_lang::LinkError;

    fn unit_fn(name: &str) -> Function {
        let mut b = Builder::new(name, &[], Type::Unit);
        b.ret(None);
        b.finish()
    }

    #[test]
    fn test_compile_single_sets_entry_to_the_function() {
        let mut b = Builder::new("inc", &[Type::Int], Type::Int);
        let x = b.block_params(b.entry())[0];
        let one = b.iconst(1);
        let y = b.bin(BinOp::Add, x, one);
        b.ret(Some(y));

        let image = compile(&b.finish()).unwrap();
        assert_eq!(image.entry(), Some(0));
        assert_eq!(image.symbol("inc"), Some(0));
        assert!(!image.section(".text").unwrap().data().is_empty());
    }

    #[test]
    fn test_compile_rejects_invalid_function() {
        let func = Builder::new("bad", &[], Type::Int).finish();
        assert!(matches!(compile(&func), Err(AotError::Codegen(_))));
    }

    #[test]
    fn test_compiler_lays_out_functions_in_add_order() {
        let mut compiler = Compiler::new();
        compiler.base_address(0x2000).entry("first");
        compiler.add(&unit_fn("first")).unwrap();
        compiler.add(&unit_fn("second")).unwrap();

        let image = compiler.link().unwrap();
        assert_eq!(image.entry(), Some(0x2000));
        assert_eq!(image.symbol("first"), Some(0x2000));
        assert!(image.symbol("second").unwrap() > 0x2000);
    }

    #[test]
    fn test_compiler_duplicate_name_is_link_error() {
        let mut compiler = Compiler::new();
        compiler.add(&unit_fn("dup")).unwrap();
        compiler.add(&unit_fn("dup")).unwrap();

        match compiler.link() {
            Err(AotError::Link(LinkError::DuplicateSymbol { name })) => assert_eq!(name, "dup"),
            other => panic!("expected duplicate-symbol link error, got {other:?}"),
        }
    }

    #[test]
    fn test_compiler_undefined_entry_is_link_error() {
        let mut compiler = Compiler::new();
        compiler.entry("missing");
        compiler.add(&unit_fn("present")).unwrap();

        assert!(matches!(
            compiler.link(),
            Err(AotError::Link(LinkError::UndefinedEntry { .. }))
        ));
    }

    #[test]
    fn test_compiler_default_has_no_entry() {
        let mut compiler = Compiler::default();
        compiler.add(&unit_fn("f")).unwrap();
        assert_eq!(compiler.link().unwrap().entry(), None);
    }
}
