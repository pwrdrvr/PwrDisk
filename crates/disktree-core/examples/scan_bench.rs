//! Headless scan timing; run each sample in a fresh process.
use std::path::PathBuf;
use std::time::Instant;

use disktree_core::scan::{ScanOptions, scan};

fn main() -> std::io::Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("scan root"));
    let mut options = ScanOptions::default();
    if std::env::var_os("BENCH_NOCLONE").is_some() {
        options.apfs_clone_metadata = false;
    }
    let started = Instant::now();
    let tree = scan(&root, options)?;
    let elapsed = started.elapsed().as_secs_f64();
    let node = tree.root();
    println!(
        "files={} dirs={} bytes={}",
        node.files, node.dirs, node.bytes
    );
    let started = Instant::now();
    drop(tree);
    println!(
        "scan_s={elapsed:.6} free_s={:.6}",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
