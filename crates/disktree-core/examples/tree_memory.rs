//! What a scanned tree costs to hold and to let go of, on macOS:
//! `cargo run --release -p disktree-core --example tree_memory -- PATH`
//!
//! Scans PATH, then reports the process's physical footprint (resident
//! plus compressed, what Activity Monitor calls Memory), the time to copy
//! and re-rank the tree as the size/files toggle does, and the time and
//! page faults of dropping each copy. Run it in a fresh process each time:
//! a drop's cost depends on what the allocator and the compressor hold.

#![allow(
    unsafe_code,
    reason = "getrusage and proc_pid_rusage, which std does not wrap"
)]
#![allow(clippy::cast_precision_loss, reason = "report figures")]

#[cfg(target_os = "macos")]
fn main() -> std::io::Result<()> {
    use std::path::PathBuf;
    use std::time::Instant;

    use disktree_core::scan::{ScanOptions, scan};
    use disktree_core::tree::{Metric, aggregate};

    let root = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    let before = footprint_mib();
    let started = Instant::now();
    let tree = scan(&root, ScanOptions::default())?;
    let arena = tree.arena_bytes();
    println!(
        "scan={:.2}s files={} dirs={} footprint+={:.0}MiB \
         arena_used={:.0}MiB arena_reserved={:.0}MiB",
        started.elapsed().as_secs_f64(),
        tree.files,
        tree.dirs,
        footprint_mib() - before,
        arena.used as f64 / MIB,
        arena.reserved as f64 / MIB,
    );

    let started = Instant::now();
    let mut copy = tree.clone();
    let cloned = started.elapsed().as_secs_f64();
    let started = Instant::now();
    aggregate(&mut copy, Metric::Files);
    println!(
        "toggle: copy={cloned:.2}s aggregate={:.2}s",
        started.elapsed().as_secs_f64()
    );
    timed_drop("copy", copy);
    timed_drop("scanned tree", tree);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("tree_memory reads macOS process counters; it runs on macOS");
}

#[cfg(target_os = "macos")]
const MIB: f64 = 1024.0 * 1024.0;

#[cfg(target_os = "macos")]
fn faults() -> (i64, i64) {
    // SAFETY: getrusage fills the struct it is given.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
    (usage.ru_minflt, usage.ru_majflt)
}

/// Resident plus compressed: plain RSS leaves out what the compressor
/// holds, which on a large tree is most of it.
#[cfg(target_os = "macos")]
fn footprint_mib() -> f64 {
    // SAFETY: proc_pid_rusage fills the v2 struct it is given.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let pid = i32::try_from(std::process::id()).expect("a pid fits");
    // SAFETY: as above.
    unsafe {
        libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, (&raw mut info).cast())
    };
    info.ri_phys_footprint as f64 / MIB
}

#[cfg(target_os = "macos")]
fn timed_drop<T>(label: &str, value: T) {
    let (minor, major) = faults();
    let started = std::time::Instant::now();
    drop(value);
    let seconds = started.elapsed().as_secs_f64();
    let (minor_after, major_after) = faults();
    println!(
        "{label}: drop={seconds:.3}s minor_faults={} major_faults={}",
        minor_after - minor,
        major_after - major
    );
}
