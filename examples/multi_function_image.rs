//! Lay several functions out into one image with a chosen base address and entry
//! point, then walk the symbol table.
//!
//! Run with:
//!
//! ```bash
//! cargo run --example multi_function_image
//! ```

use aot_lang::Compiler;
use ir_lang::{BinOp, Builder, Function, Type};

/// `fn <name>(a: int, b: int) -> int { a <op> b }`
fn binary(name: &str, op: BinOp) -> Function {
    let mut b = Builder::new(name, &[Type::Int, Type::Int], Type::Int);
    let a = b.block_params(b.entry())[0];
    let c = b.block_params(b.entry())[1];
    let r = b.bin(op, a, c);
    b.ret(Some(r));
    b.finish()
}

fn main() {
    let mut compiler = Compiler::new();
    // Lay the image out at a typical load address, entered at `add`.
    compiler.base_address(0x0040_0000).entry("add");
    compiler.add(&binary("add", BinOp::Add)).unwrap();
    compiler.add(&binary("sub", BinOp::Sub)).unwrap();
    compiler.add(&binary("mul", BinOp::Mul)).unwrap();

    let image = compiler
        .link()
        .expect("names are distinct and the entry exists");

    println!("entry = {:#018x}", image.entry().unwrap());
    println!("symbols:");
    for (name, address) in image.symbols() {
        println!("  {address:#018x}  {name}");
    }
    println!("\n.text is {} bytes", image.section(".text").unwrap().len());
}
