//! Compile a single function ahead of time and inspect the resulting image.
//!
//! Run with:
//!
//! ```bash
//! cargo run --example compile_and_inspect
//! ```

use aot_lang::compile;
use ir_lang::{BinOp, Builder, Type};

fn main() {
    // fn poly(x: int) -> int { x * x + x }
    let mut b = Builder::new("poly", &[Type::Int], Type::Int);
    let x = b.block_params(b.entry())[0];
    let sq = b.bin(BinOp::Mul, x, x);
    let sum = b.bin(BinOp::Add, sq, x);
    b.ret(Some(sum));

    let image = compile(&b.finish()).expect("poly is well-formed");

    // The image places `poly` at the entry point and holds its object code in `.text`.
    println!("entry     = {:?}", image.entry());
    println!("poly @    = {:?}", image.symbol("poly"));
    println!(
        "code size = {} bytes",
        image.section(".text").unwrap().len()
    );
    println!();

    // `Image` renders a readable link map.
    println!("{image}");
}
