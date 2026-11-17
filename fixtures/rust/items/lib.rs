//! Crate docs are not attached to a symbol.

use std::collections::{HashMap, hash_map::Entry as E};
use std::fmt::{self, Display};
use crate::store::*;
pub use self::shapes::Circle;
extern crate alloc as a;

mod store;
pub mod shapes {
    //! Shapes live here.

    use super::Area;

    /// A circle.
    #[derive(Debug, Clone)]
    pub struct Circle {
        pub r: f64,
    }

    impl Area for Circle {
        fn area(&self) -> f64 {
            self.r * self.r * 3.0
        }
    }

    mod nested;
}

pub const PI: f64 = 3.14;
static mut COUNT: u32 = 0;
pub(crate) type Map = HashMap<String, Circle>;

/// Things with an area.
pub trait Area {
    fn area(&self) -> f64;
    /// Twice the area.
    fn double(&self) -> f64 {
        self.area() * 2.0
    }
}

pub enum Shape {
    Round(Circle),
    Square { side: f64 },
}

impl Shape {
    pub fn new(r: f64) -> Self {
        Shape::Round(Circle { r })
    }

    pub async fn load<'a>(&mut self, mut n: u32, (a, b): (u8, u8)) -> Map
    where
        Self: Sized,
    {
        let m = Map::new();
        helper(n);
        println!("{}", total(&m).len());
        m
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", PI)
    }
}

fn helper(x: u32) -> u32 {
    fn inner() {}
    inner();
    x.pow(2)
}

fn total(m: &Map) -> Vec<f64> {
    m.values().map(|c| c.area()).collect::<Vec<_>>()
}
