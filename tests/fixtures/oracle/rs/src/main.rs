use demo::{total, Square};
use std::collections::HashMap;

fn main() {
    let s = Square::new(2.0);
    let _m: HashMap<u8, u8> = HashMap::new();
    println!("{}", total(&[&s]));
}
