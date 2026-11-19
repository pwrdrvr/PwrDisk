#![cfg(target_os = "macos")]

use disktree_core::space::{display_space_info, refresh_display_space_info};
use std::path::Path;

#[test]
#[ignore = "opt in to real CacheDelete on the startup volume"]
fn important_capacity_on_the_startup_volume() {
    let root = Path::new("/");
    let cold = display_space_info(root).expect("read startup volume");
    assert_eq!(cold.reclaimable_available(), None);

    let sample = refresh_display_space_info(root).expect("read native space");
    // macOS may withhold the estimate. Raw capacity is still available.
    assert!(sample.available <= sample.total);
    if let Some(available) = sample.native.and_then(|native| native.available) {
        assert!((sample.available..=sample.total).contains(&available));
    }

    // A second refresh uses the same budget and should hit the cache.
    let repeated =
        refresh_display_space_info(root).expect("reuse native space");
    assert!(repeated.available <= repeated.total);
}
