use super::super::util::helper;

/// The thing.
pub struct Thing;

impl Thing {
    pub fn make() -> Self {
        helper();
        Thing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::b;

    #[test]
    fn makes() {
        let _ = Thing::make();
    }
}
