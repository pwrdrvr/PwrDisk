//! A removal must account for each scanned file once, even when later
//! removals enclose earlier ones. The oracle keeps files, not a removal
//! ledger, so it cannot lose bytes by compacting nested removal records.

use std::collections::BTreeSet;

use disktree_link::{FindRequest, Request, Response};
use proptest::prelude::*;
use proptest::test_runner::{TestCaseResult, TestRunner};

use super::*;

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
enum Command {
    Remove(usize),
    Mark(usize),
    Unmark(usize),
    Rescan,
    Metric,
}

struct Model {
    paths: Vec<PathBuf>,
    own: Vec<u64>,
    live: Vec<bool>,
    scanned: Vec<bool>,
    marked: BTreeSet<PathBuf>,
}

impl Model {
    fn new(root: &Path, entries: &[(usize, u64)]) -> Self {
        let mut paths = vec![root.to_path_buf()];
        let mut own = vec![0];
        for (index, &(parent, bytes)) in entries.iter().enumerate() {
            let name = if index == 0 {
                ".cache".into()
            } else {
                format!("n{index}")
            };
            let path = paths[parent % paths.len()].join(name);
            std::fs::create_dir(&path).unwrap();
            // Sparse files reach the insight threshold without allocating
            // hundreds of MiB for each apparent-size model case.
            std::fs::File::create(path.join("payload"))
                .unwrap()
                .set_len(bytes)
                .unwrap();
            paths.push(path);
            own.push(bytes);
        }
        Self {
            live: vec![true; paths.len()],
            scanned: vec![true; paths.len()],
            paths,
            own,
            marked: BTreeSet::new(),
        }
    }

    fn bytes(&self, path: &Path, removed: bool) -> u64 {
        self.paths
            .iter()
            .enumerate()
            .filter(|&(i, file_dir)| {
                file_dir.starts_with(path)
                    && self.scanned[i]
                    && (!removed || !self.live[i])
            })
            .map(|(i, _)| self.own[i])
            .sum()
    }

    fn check(
        &self,
        app: &mut Disktree,
        cx: &mut Context<'_, Disktree>,
    ) -> TestCaseResult {
        for (index, path) in self.paths.iter().enumerate() {
            let crumbs = app.crumbs_for_path(path);
            prop_assert_eq!(crumbs.is_some(), self.scanned[index]);
            let Some(crumbs) = crumbs else { continue };
            let scanned = self.bytes(path, false);
            let removed = self.bytes(path, true);
            if self.live[index] {
                prop_assert_eq!(
                    app.target_at(&crumbs).unwrap().bytes,
                    scanned - removed,
                    "target at {:?}",
                    path
                );
                prop_assert_eq!(
                    app.removed.below(path, app.path_style()),
                    removed,
                    "removed below {:?}",
                    path
                );
            }
            let Response::Listing(listing) = app.answer_agent(
                &Request::Ls {
                    path: path.clone(),
                    limit: 0,
                },
                cx,
            ) else {
                panic!("directory listing");
            };
            prop_assert_eq!(listing.entry.bytes, scanned);
            prop_assert_eq!(listing.entry.removed, !self.live[index]);
            let expected = if self.live[index] { removed } else { 0 };
            prop_assert_eq!(
                listing.entry.removed_bytes,
                expected,
                "ls at {:?}",
                path
            );
            // Search returns topmost matches only. Starting at the parent
            // makes this directory a match without an enclosing match.
            if index != 0 {
                let Response::Found(found) = app.answer_agent(
                    &Request::Find(FindRequest {
                        path: path.parent().map(Path::to_path_buf),
                        kind: "dir".into(),
                        limit: self.paths.len(),
                        ..FindRequest::default()
                    }),
                    cx,
                ) else {
                    panic!("directory search");
                };
                let hit = found
                    .matches
                    .iter()
                    .find(|hit| hit.path == *path)
                    .expect("every scanned directory remains searchable");
                prop_assert_eq!(hit.entry.removed, !self.live[index]);
                prop_assert_eq!(
                    hit.entry.removed_bytes,
                    expected,
                    "find at {:?}",
                    path
                );
            }
        }
        let marked: BTreeSet<_> = app
            .marks
            .items()
            .iter()
            .map(|item| item.path.clone())
            .collect();
        prop_assert_eq!(&marked, &self.marked);
        for target in app.marks.items() {
            prop_assert_eq!(
                target.bytes,
                self.bytes(&target.path, false)
                    - self.bytes(&target.path, true),
                "mark at {:?}",
                target.path
            );
        }
        let expected = self.insights(app.options.metric);
        let insights: Vec<_> = app
            .insights
            .iter()
            .map(|candidate| {
                (app.path_at(&candidate.crumbs).unwrap(), candidate.bytes)
            })
            .collect();
        prop_assert_eq!(
            insights,
            expected,
            "insights count live scanned bytes"
        );
        Ok(())
    }

    fn insights(&self, metric: Metric) -> Vec<(PathBuf, u64)> {
        // Every top-level directory inherits the fixture's explicit tmp
        // context; .cache has its own hint. Findings are topmost only.
        let mut eligible: Vec<_> = self
            .paths
            .iter()
            .enumerate()
            .filter(|&(i, path)| {
                self.scanned[i] && path.parent() == Some(&self.paths[0])
            })
            .filter_map(|(i, path)| {
                let scanned = self.bytes(path, false);
                let files = self
                    .paths
                    .iter()
                    .enumerate()
                    .filter(|&(j, child)| {
                        self.scanned[j] && child.starts_with(path)
                    })
                    .count();
                (scanned >= 64 * MIB).then_some((i, scanned, files))
            })
            .collect();
        // Rank by scanned size before discounting removals. Equal sizes
        // retain tree order: file count in that metric, then name.
        eligible.sort_by_key(|&(i, scanned, files)| {
            (
                std::cmp::Reverse(scanned),
                std::cmp::Reverse(if metric == Metric::Files {
                    files
                } else {
                    0
                }),
                &self.paths[i],
            )
        });
        eligible.truncate(INSIGHT_LIMIT);
        eligible
            .into_iter()
            .filter_map(|(i, scanned, _)| {
                let path = &self.paths[i];
                let remaining = scanned - self.bytes(path, true);
                // A finding remains below the threshold until a rescan.
                (self.live[i] && remaining > 0)
                    .then(|| (path.clone(), remaining))
            })
            .collect()
    }

    fn step(
        &mut self,
        command: &Command,
        app: &mut Disktree,
        cx: &mut Context<'_, Disktree>,
    ) {
        match *command {
            Command::Remove(i) | Command::Mark(i) | Command::Unmark(i) => {
                let i = 1 + i % (self.paths.len() - 1);
                let path = self.paths[i].clone();
                if matches!(command, Command::Unmark(_)) {
                    app.unmark(&path, cx);
                    self.marked.remove(&path);
                } else if self.live[i] {
                    let crumbs = app.crumbs_for_path(&path).unwrap();
                    if matches!(command, Command::Remove(_)) {
                        // Stand in only for a successful worker report. The
                        // real target and ledger methods remain under test.
                        let target = app.target_at(&crumbs).unwrap();
                        std::fs::remove_dir_all(&path).unwrap();
                        for (child, live) in
                            self.paths.iter().zip(&mut self.live)
                        {
                            if child.starts_with(&path) {
                                *live = false;
                            }
                        }
                        app.marks.clear();
                        self.marked.clear();
                        app.note_removed(vec![target]);
                    } else {
                        app.toggle_mark(&crumbs, cx);
                        if !self.marked.remove(&path)
                            && !self.marked.iter().any(|p| path.starts_with(p))
                        {
                            self.marked.retain(|p| !p.starts_with(&path));
                            self.marked.insert(path);
                        }
                    }
                }
            }
            Command::Rescan => {
                app.start_scan(cx);
                let deadline = Instant::now() + Duration::from_secs(5);
                while app.poll_scan_once(app.scan_epoch, cx) {
                    assert!(Instant::now() < deadline, "scan timed out");
                    std::thread::sleep(Duration::from_millis(1));
                }
                self.scanned.clone_from(&self.live);
            }
            Command::Metric => app.toggle_metric(cx),
        }
    }
}

fn run(
    entries: &[(usize, u64)],
    commands: &[Command],
    cx: &gpui_kit::TestAppContext,
) -> TestCaseResult {
    // macOS's default temp directory is inside the protected /var/folders.
    // Use the scratch tree the removal guards explicitly permit instead.
    #[cfg(unix)]
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    #[cfg(not(unix))]
    let temp = tempfile::tempdir().unwrap();
    // Make inherited temporary context explicit on every host, including
    // machines whose configured temp directory has an unrelated name.
    let root = temp.path().join("tmp");
    std::fs::create_dir(&root).unwrap();
    let root = dunce::canonicalize(root).unwrap();
    let mut model = Model::new(&root, entries);
    let options = ScanOptions {
        apparent_size: true,
        ..ScanOptions::default()
    };
    let tree = disktree_core::scan::scan(&root, options.clone()).unwrap();
    let view = cx.update(|cx| {
        cx.new(|cx| {
            // No initial scan or window: supply the tree measured above.
            let mut app =
                Disktree::build(root, options, 3, None, false, false, cx);
            app.close_volumes(cx);
            app.awaiting_root = false;
            app.tree = Some(Arc::new(tree));
            app.refresh_insights();
            // Keep apparent-size accounting independent of the host disk's
            // capacity; this model does not exercise the removal limit.
            app.disk_probe = |_| {
                Ok(crate::mark_guard::Disk {
                    key: "model".into(),
                    capacity: 1 << 40,
                    name: "model".into(),
                })
            };
            app.removal_settings_dir = None;
            app.power_settings_path = None;
            app
        })
    });
    cx.update(|cx| {
        view.update(cx, |app, cx| {
            model.check(app, cx)?;
            for command in commands {
                model.step(command, app, cx);
                model.check(app, cx)?;
            }
            Ok(())
        })
    })
}

#[gpui_kit::test]
fn removals_and_marks_follow_the_scanned_file_model(
    cx: &gpui_kit::TestAppContext,
) {
    let commands = prop::collection::vec(
        prop_oneof![
            (0_usize..16).prop_map(Command::Remove),
            (0_usize..16).prop_map(Command::Mark),
            (0_usize..16).prop_map(Command::Unmark),
            Just(Command::Rescan),
            Just(Command::Metric),
        ],
        1..50,
    );
    let mut runner = TestRunner::new(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    });
    runner
        .run(
            &(
                prop::collection::vec(
                    (0_usize..16, (0_u64..97).prop_map(|mib| mib * MIB)),
                    3..12,
                ),
                commands,
            ),
            |(entries, commands)| run(&entries, &commands, cx),
        )
        .expect("removed bytes equal the scanned files removed exactly once");
}

#[gpui_kit::test]
fn temporary_siblings_remain_insights_after_removing_the_cache(
    cx: &gpui_kit::TestAppContext,
) {
    run(
        &[(0, 0), (0, 64 * MIB), (0, 0)],
        &[Command::Remove(0), Command::Metric, Command::Rescan],
        cx,
    )
    .unwrap();
}

#[gpui_kit::test]
fn nested_removals_discount_insights_from_the_scanned_size(
    cx: &gpui_kit::TestAppContext,
) {
    // .cache = 100 MiB, b = 60 MiB, c = 20 MiB. The insight must show
    // 40 MiB after removing c then b, just like a mark of .cache.
    run(
        &[(0, 40 * MIB), (1, 40 * MIB), (2, 20 * MIB)],
        &[
            Command::Remove(2),
            Command::Remove(1),
            Command::Mark(0),
            Command::Unmark(0),
            Command::Metric,
            Command::Rescan,
        ],
        cx,
    )
    .unwrap();
}

#[gpui_kit::test]
fn enclosing_removal_keeps_the_bytes_of_an_earlier_inner_removal(
    cx: &gpui_kit::TestAppContext,
) {
    // a = 100, a/b = 60, a/b/c = 20. Removing c then b leaves 40 at a.
    run(
        &[(0, 40), (1, 40), (2, 20)],
        &[
            Command::Remove(2),
            Command::Mark(1),
            Command::Unmark(1),
            Command::Remove(1),
            Command::Mark(0),
            Command::Metric,
            Command::Rescan,
        ],
        cx,
    )
    .unwrap();
}
