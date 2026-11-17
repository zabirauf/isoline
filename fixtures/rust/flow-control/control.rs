use std::collections::HashMap;
use crate::store::{load, Store};

pub fn classify(n: i32) -> &'static str {
    if n < 0 {
        log("negative");
        return "neg";
    } else if n == 0 {
        "zero"
    } else {
        log("positive");
        "pos"
    }
}

fn find(map: &HashMap<String, u32>, key: &str) -> Option<u32> {
    let value = *map.get(key)?;
    if let Some(limit) = load(key) {
        check(limit)?;
    }
    match value {
        0 => None,
        v if v > 10 => Some(scale(v)),
        v => {
            log("small");
            Some(v)
        }
    }
}

fn drain(store: &mut Store) -> usize {
    let mut count = 0;
    loop {
        if store.is_empty() {
            break;
        }
        count += 1;
    }
    while let Some(item) = store.pop() {
        handle(item);
    }
    for (i, x) in store.iter().enumerate() {
        println!("{} {}", i, render(x));
    }
    if count == 0 {
        panic!("nothing drained");
    }
    count
}

fn log(msg: &str) {
    eprintln!("{msg}");
}
