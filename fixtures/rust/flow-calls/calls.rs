use std::sync::Arc;
use serde_json::{self, Value};

pub struct Repo {
    items: Vec<u32>,
}

impl Repo {
    pub fn new() -> Self {
        Self::with_items(Vec::new())
    }

    fn with_items(items: Vec<u32>) -> Self {
        Repo { items }
    }

    pub async fn sync(&self, client: &Client) -> Result<usize, Error> {
        let body = client.fetch(&self.url()).await?;
        let parsed: Value = serde_json::from_str(&body)?;
        let total: u32 = self.items.iter().map(|x| weigh(*x)).sum();
        self.items.iter().for_each(|x| {
            if *x > 9 {
                warn(*x);
            }
        });
        Ok(total as usize + parsed.as_u64().unwrap_or(0) as usize)
    }

    fn url(&self) -> String {
        format!("/repo/{}", self.items.len())
    }
}

pub trait Named {
    fn name(&self) -> String;

    fn greet(&self) -> String {
        let shout = |s: String| s.to_uppercase();
        shout(self.name())
    }
}

fn weigh(x: u32) -> u32 {
    fn square(v: u32) -> u32 {
        v * v
    }
    let shared = Arc::new(x);
    crate::metrics::record(*shared);
    square(x) + Repo::new().items.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighs() {
        assert_eq!(weigh(2), 4);
    }
}
