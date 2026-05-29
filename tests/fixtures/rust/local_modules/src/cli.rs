use crate::util::helper;
use std::fmt;

pub struct Command;
pub struct Flag;

pub fn run() {
    helper();
    let _ = std::any::type_name::<fmt::Error>();
}
