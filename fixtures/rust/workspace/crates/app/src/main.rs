mod cli;
mod missing;

use core_lib::a::b::Thing;
use core_lib::Thing as Reexported;
use shapes::a;
use crate::cli::run;
use cli::run as run2;
use std::fmt;
use serde::Serialize;

fn main() {
    run();
    run2();
    let _ = (Thing::make(), fmt::Error);
}
