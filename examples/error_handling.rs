//! Show how the two failure stages surface: a function that does not validate is a
//! code-generation error, and a name collision is a link error.
//!
//! Run with:
//!
//! ```bash
//! cargo run --example error_handling
//! ```

use aot_lang::{AotError, Compiler, compile};
use core::error::Error;
use ir_lang::{Builder, Type};

fn unit(name: &str) -> ir_lang::Function {
    let mut b = Builder::new(name, &[], Type::Unit);
    b.ret(None);
    b.finish()
}

fn main() {
    // Stage one: the function promises an int return but never returns one, so it
    // fails to validate and is rejected during lowering.
    let invalid = Builder::new("bad", &[], Type::Int).finish();
    match compile(&invalid) {
        Err(err @ AotError::Codegen(_)) => {
            println!("codegen error: {err}");
            if let Some(source) = err.source() {
                println!("  caused by: {source}");
            }
        }
        other => panic!("expected a codegen error, got {other:?}"),
    }

    // Stage two: two well-formed functions share a name, so the link fails.
    let mut compiler = Compiler::new();
    compiler.add(&unit("dup")).unwrap();
    compiler.add(&unit("dup")).unwrap();
    match compiler.link() {
        Err(err @ AotError::Link(_)) => {
            println!("link error: {err}");
            if let Some(source) = err.source() {
                println!("  caused by: {source}");
            }
        }
        other => panic!("expected a link error, got {other:?}"),
    }
}
