use std::thread;
use anyhow::{bail, Context};

pub fn spawn(jobs: Vec<Job>) -> anyhow::Result<()> {
    let Some(first) = jobs.first() else {
        bail!("no jobs");
    };
    let handle = thread::spawn(move || run(first));
    let name = first.name().unwrap_or_else(|| fallback());
    let on_exit = move || {
        cleanup(&jobs);
    };
    let list = vec![summary(first), util::render(stats(&jobs))];
    handle.join().ok().context("join")?;
    Ok(())
}
