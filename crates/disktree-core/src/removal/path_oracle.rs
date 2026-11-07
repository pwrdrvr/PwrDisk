//! Removal eligibility belongs to directory entries, not their spelling.
//! This oracle uses stat identities and ancestry, never guard keys.

use super::*;
use crate::test_paths::{id, spellings};
use proptest::prelude::*;
use std::os::unix::fs::symlink;

fn ancestors(path: &Path) -> Vec<(u64, u64)> {
    let real = path.canonicalize().expect("existing ancestor");
    real.ancestors().filter_map(id).collect()
}

fn contains(path: &Path, other: &Path) -> bool {
    id(path).is_some_and(|id| ancestors(other).contains(&id))
}

fn oracle(
    path: &Path,
    root: &Path,
    home: Option<&Path>,
    mounts: &[Vec<(u64, u64)>],
) -> bool {
    let root = root.canonicalize().expect("root");
    // Resolve only ancestors: marking a link unlinks it, even when its
    // target is protected, missing or a loop.
    let parent = path
        .parent()
        .expect("parent")
        .canonicalize()
        .expect("parent");
    let entry = parent.join(path.file_name().expect("entry name"));
    let entry_id = id(&entry).expect("entry");
    let meta = fs::symlink_metadata(&entry).expect("entry metadata");
    let entry_ancestors = if meta.is_dir() {
        ancestors(&entry)
    } else {
        let mut ids = ancestors(&parent);
        ids.push(entry_id);
        ids
    };
    if !entry_ancestors.contains(&id(&root).expect("root identity"))
        || id(&root) == Some(entry_id)
    {
        return true;
    }
    if let Some(home) = home
        && ancestors(home).contains(&entry_id)
    {
        return true;
    }
    let profiles = if cfg!(target_os = "macos") {
        "/Users"
    } else {
        "/home"
    };
    if id(Path::new(profiles)) == Some(entry_id)
        || (meta.is_dir() && id(&parent) == id(Path::new(profiles)))
    {
        return true;
    }
    let in_home = home.is_some_and(|home| {
        ancestors(home)
            .first()
            .is_some_and(|home| entry_ancestors.contains(home))
    });
    let in_scratch = SCRATCH_TREES.iter().any(|scratch| {
        id(Path::new(scratch)).is_some_and(|scratch| {
            scratch != entry_id && entry_ancestors.contains(&scratch)
        })
    });
    if !in_home
        && !in_scratch
        && SYSTEM_TREES.iter().any(|system| {
            id(Path::new(system))
                .is_some_and(|system| entry_ancestors.contains(&system))
        })
    {
        return true;
    }
    if meta.is_dir()
        && SYSTEM_TREES.iter().any(|system| {
            let system = Path::new(system);
            system.exists() && contains(&entry, system)
        })
    {
        return true;
    }
    // Read the actual namespace, bypassing removal's mount cache. Identity
    // ancestry catches a mount below a target reached through any alias.
    mounts.iter().any(|mount| {
        mount.first() == Some(&entry_id)
            || (meta.is_dir() && mount.contains(&entry_id))
    })
}

fn check(root: &Path, relative: &[PathBuf], link: &Path, home: Option<&Path>) {
    let mounts = read_mount_table();
    let mount_ids: Vec<_> = crate::space::mount_boundaries_for(Path::new("/"))
        .expect("real mount table")
        .iter()
        .filter(|mount| mount.exists())
        .map(|mount| ancestors(mount))
        .collect();
    let real_root = root.canonicalize().expect("root");
    let home_guard = home.map(Home::of);
    for spelling in spellings(root, link) {
        for relative in relative {
            let path = spelling.join(relative);
            let expected = oracle(&path, &spelling, home, &mount_ids);
            let reason = judge(
                &path,
                &spelling,
                Some(&real_root),
                home_guard.as_ref(),
                &mounts,
            );
            assert_eq!(
                reason.is_some(),
                expected,
                "{} in {}: {reason:?}",
                path.display(),
                spelling.display()
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn removal_verdict_matches_identity_under_every_root_spelling(
        location in 0_u8..3,
        names in prop::collection::vec("[a-z]{1,8}", 1..6),
        directory_links in any::<bool>(),
    ) {
        let aliases = tempfile::tempdir().expect("aliases");
        let temp = match location {
            0 => tempfile::tempdir_in(std::env::current_dir().expect("cwd")),
            1 => tempfile::tempdir(),
            _ => tempfile::tempdir_in("/tmp"),
        }.expect("tree");
        let root = temp.path().canonicalize().expect("root");
        let mut paths = vec![PathBuf::new()];
        for (index, name) in names.iter().enumerate() {
            let dir = PathBuf::from(format!("{index}-{name}"));
            fs::create_dir(root.join(&dir)).expect("directory");
            fs::write(root.join(&dir).join("file"), name).expect("file");
            let link = dir.join("link");
            let target = if directory_links {
                root.clone()
            } else {
                root.join(&dir).join("file")
            };
            symlink(target, root.join(&link)).expect("link");
            symlink("missing", root.join(&dir).join("dangling"))
                .expect("dangling");
            symlink("loop", root.join(&dir).join("loop")).expect("loop");
            paths.extend([dir.clone(), dir.join("file"), link,
                dir.join("dangling"), dir.join("loop")]);
        }
        check(
            temp.path(), &paths, &aliases.path().join("root"),
            std::env::home_dir().as_deref(),
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn var_folders_is_protected_even_when_the_root_is_a_link() {
    let temp = tempfile::tempdir().expect("protected fixture");
    fs::write(temp.path().join("file"), b"x").expect("file");
    let aliases = tempfile::tempdir().expect("aliases");
    check(
        temp.path(),
        &["file".into()],
        &aliases.path().join("root"),
        None,
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_and_profile_entries_keep_their_verdict_through_a_root_alias() {
    let aliases = tempfile::tempdir().expect("aliases");
    for (index, (root, entry)) in [
        ("/private/var", "db/receipts"),
        ("/", "Library/Caches"),
        ("/", "Users/Shared"),
    ]
    .iter()
    .enumerate()
    {
        check(
            Path::new(root),
            &[entry.into()],
            &aliases.path().join(index.to_string()),
            None,
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn a_home_under_var_keeps_its_system_exemption_under_its_real_name() {
    let temp = tempfile::tempdir().expect("home under protected var");
    fs::create_dir_all(temp.path().join("home/Library")).expect("home");
    let real_home = temp.path().join("home").canonicalize().expect("home");
    let aliases = tempfile::tempdir().expect("aliases");
    let home = aliases.path().join("home");
    symlink(&real_home, &home).expect("home alias");
    // The same topology as HOME=/var/root, without probing root's private
    // Library or mutating the test process's environment.
    check(
        &real_home,
        &["Library".into()],
        &aliases.path().join("root"),
        Some(&home),
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_real_mount_below_a_target_is_refused_under_every_root_spelling() {
    let temp = tempfile::tempdir_in("/tmp").expect("tree");
    let root = temp.path().canonicalize().expect("root");
    let _image = crate::test_paths::DiskImage::mount_at(&root.join("a/mnt"));
    let aliases = tempfile::tempdir().expect("aliases");
    check(
        &root,
        &["a".into(), "a/mnt".into()],
        &aliases.path().join("root"),
        None,
    );
}
