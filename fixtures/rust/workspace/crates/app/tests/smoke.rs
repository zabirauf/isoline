mod common;

use core_lib::Thing;

#[test]
fn smoke() {
    common::setup();
    let _ = Thing::make();
}
