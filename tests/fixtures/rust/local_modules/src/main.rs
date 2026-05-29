mod cli;
mod util;

use cli::{Command, Flag};
use util::helper;
use std::collections::{HashMap, HashSet};
use serde::Serialize;

fn main() {
    let _ = (Command, Flag);
    let _: HashMap<String, HashSet<u8>> = HashMap::new();
    helper();
}
