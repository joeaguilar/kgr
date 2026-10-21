pub mod net;
mod util;

use serde::Serialize;

pub trait Shape {
    fn area(&self) -> f64;
    fn name(&self) -> String {
        String::from("shape")
    }
}

pub struct Square<T> {
    pub side: T,
}

impl Shape for Square<f64> {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

impl<T> Square<T> {
    pub fn new(side: T) -> Self {
        Square { side }
    }
}

pub fn total(shapes: &[&dyn Shape]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}

#[derive(Serialize)]
pub struct Dto;
