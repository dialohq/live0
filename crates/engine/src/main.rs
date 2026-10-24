#![allow(clippy::unwrap_used)]

use std::env::args;

fn main() {
    println!("engine!");

    let mut args = args();
    args.next();
    let query = args.next().unwrap();
    let parsed = parser::parse(&query).unwrap();

    println!("{parsed:?}");
}
