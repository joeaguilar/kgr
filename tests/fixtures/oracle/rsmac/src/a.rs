use crate::{in_macro, plain};
pub fn user() {
    println!("{}", in_macro());
    let _ = plain();
}
