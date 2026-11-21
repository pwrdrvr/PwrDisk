//! "Worth a look": the biggest things in a scan that could plausibly go.
//!
//! Three kinds of finding, each one a directory a person can judge in a
//! second: reclaimable space ([`crate::classify::Reclaim`]), agent worktrees,
//! and experiments nobody has written to in a month. Findings never nest, so
//! their total is space that really exists once.

use crate::classify::{Category, Reclaim};
use crate::tree::Node;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// An experiment untouched this long is worth a look.
pub const STALE_DAYS: i64 = 30;

/// Smaller than this is not worth a line.
const MIN_BYTES: u64 = 64 * 1024 * 1024;

/// Why a directory is on the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// Its space can be had back, for this reason.
    Reclaimable(Reclaim),
    /// A directory of agent worktrees: how many, and the oldest one's age.
    Worktrees { count: usize, oldest_days: i64 },
    /// Experiments untouched for [`STALE_DAYS`]: only those are counted.
    StaleExperiments { count: usize },
}

impl Finding {
    /// The finding in a few words, as a list reads it after the size.
    pub fn describe(&self) -> String {
        match self {
            Self::Reclaimable(reclaim) => {
                format!("cleanup candidate: {}", reclaim.label())
            }
            Self::Worktrees { count, oldest_days } => format!(
                "{count} agent worktree{}, the oldest {oldest_days} d old",
                if *count == 1 { "" } else { "s" }
            ),
            Self::StaleExperiments { count } => format!(
                "{count} experiment{} untouched for a month or more",
                if *count == 1 { "" } else { "s" }
            ),
        }
    }
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Where it is, from the scanned root.
    pub crumbs: Vec<usize>,
    /// What clearing it frees. For stale experiments, only the stale ones.
    pub bytes: u64,
    /// The part of `bytes` it holds alone ([`Node::private_bytes`]), for
    /// sizes shown as private.
    pub private: u64,
    pub finding: Finding,
}

/// Which of a finding's figures decides whether it makes the list and
/// where it ranks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weighing {
    /// Its allocated bytes ([`Candidate::bytes`]).
    Allocated,
    /// What it holds alone ([`Candidate::private`]), so a folder of clones
    /// whose blocks live on elsewhere gives its place to one whose removal
    /// would hand space back.
    Private,
}

impl Weighing {
    /// `candidate`'s figure under this weighing.
    pub const fn of(self, candidate: &Candidate) -> u64 {
        match self {
            Self::Allocated => candidate.bytes,
            Self::Private => candidate.private,
        }
    }
}

/// The `limit` largest findings beneath `root`, largest first.
///
/// `weighing` names the figure that sizes them, and one below `MIN_BYTES`
/// (64 MiB) by it is left off. Equal ones keep the order they were found in. `now` is Unix
/// seconds, passed in so the answer is testable.
pub fn worth_a_look(
    root: Node<'_>,
    now: i64,
    limit: usize,
    weighing: Weighing,
) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut crumbs = Vec::new();
    for (index, child) in root.children().iter().enumerate() {
        crumbs.push(index);
        visit(child, &mut crumbs, now, &mut found);
        crumbs.pop();
    }
    found.retain(|candidate| weighing.of(candidate) >= MIN_BYTES);
    found.sort_by_key(|candidate| std::cmp::Reverse(weighing.of(candidate)));
    found.truncate(limit);
    found
}

fn visit(
    node: Node<'_>,
    crumbs: &mut Vec<usize>,
    now: i64,
    found: &mut Vec<Candidate>,
) {
    // Nothing beneath a small directory can reach `MIN_BYTES` either, so a
    // scan of millions of files looks at a few thousand directories. Sound
    // under either weighing: what a node holds alone never exceeds its
    // allocation.
    if !node.is_dir() || node.bytes < MIN_BYTES {
        return;
    }
    // Topmost only: everything beneath a reclaimable directory goes with it.
    if let Some(reason) = node.reclaim {
        found.push(Candidate {
            crumbs: crumbs.clone(),
            bytes: node.bytes,
            private: node.private_bytes(),
            finding: Finding::Reclaimable(reason),
        });
        return;
    }
    let name = node.name();
    let scratch = node.category == Category::AgentScratch;
    if scratch && name.eq_ignore_ascii_case("worktrees") {
        let trees: Vec<Node<'_>> = node
            .children()
            .iter()
            .filter(|child| child.is_dir())
            .collect();
        if !trees.is_empty() {
            let oldest = trees
                .iter()
                .map(|tree| tree.modified)
                .filter(|&time| time > 0)
                .min()
                .unwrap_or(now);
            found.push(Candidate {
                crumbs: crumbs.clone(),
                bytes: node.bytes,
                private: node.private_bytes(),
                finding: Finding::Worktrees {
                    count: trees.len(),
                    oldest_days: (now - oldest).max(0) / DAY,
                },
            });
            return;
        }
    }
    let experiments = scratch
        && (name.eq_ignore_ascii_case("tries")
            || name.eq_ignore_ascii_case("experiments"));
    let mut stale = (0_usize, 0_u64, 0_u64);
    for (index, child) in node.children().iter().enumerate() {
        let is_stale = experiments
            && child.is_dir()
            && child.modified > 0
            && now - child.modified > STALE_DAYS * DAY;
        if is_stale {
            stale.0 += 1;
            stale.1 += child.bytes;
            stale.2 += child.private_bytes();
            // A stale experiment is judged whole; its caches go with it.
            continue;
        }
        crumbs.push(index);
        visit(child, crumbs, now, found);
        crumbs.pop();
    }
    if stale.0 > 0 {
        found.push(Candidate {
            crumbs: crumbs.clone(),
            bytes: stale.1,
            private: stale.2,
            finding: Finding::StaleExperiments { count: stale.0 },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::sharing::{Allocation, CloneInfo};
    use crate::tree::{Draft, Tree};
    use crate::tree::{Metric, NodeKind, aggregate};
    use proptest::prelude::*;

    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    const NOW: i64 = 1_800_000_000;

    fn file(name: &str, bytes: u64, days_old: i64) -> Draft {
        let mut node = Draft::entry(name, NodeKind::File, bytes);
        node.modified = NOW - days_old * DAY;
        node
    }

    /// An APFS clone of `bytes` that holds `private` of them alone; the
    /// rest it shares with a copy outside the scan. `id` names its group.
    fn clone(name: &str, bytes: u64, private: u64, id: u64) -> Draft {
        let mut node = file(name, bytes, 1);
        node.allocation = Some(Box::new(Allocation {
            bytes,
            identity: None,
            links: 1,
            followed: false,
            clone: Some(CloneInfo {
                device: 1,
                id,
                references: 2,
                private_bytes: Some(private),
                data_bytes: bytes,
            }),
        }));
        node
    }

    fn dir(name: &str, children: Vec<Draft>) -> Draft {
        let mut node = Draft::directory(name);
        node.children = children;
        node
    }

    fn scan(root: Draft) -> Tree {
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        classify(&mut root);
        root
    }

    fn home() -> Tree {
        scan(dir(
            "tobi",
            vec![
                dir(
                    ".cache",
                    vec![dir("kache", vec![file("blob", 5 * GIB, 1)])],
                ),
                dir(
                    ".codex",
                    vec![dir(
                        "worktrees",
                        vec![
                            dir("a1", vec![file("x", 3 * GIB, 41)]),
                            dir("b2", vec![file("y", 2 * GIB, 3)]),
                        ],
                    )],
                ),
                dir(
                    "src",
                    vec![dir(
                        "tries",
                        vec![
                            dir("old", vec![file("z", 4 * GIB, 90)]),
                            dir(
                                "fresh",
                                vec![
                                    file("Cargo.toml", 1, 1),
                                    dir(
                                        "target",
                                        vec![dir(
                                            "debug",
                                            vec![dir(
                                                "deps",
                                                vec![file("o", GIB, 1)],
                                            )],
                                        )],
                                    ),
                                ],
                            ),
                        ],
                    )],
                ),
                dir("Documents", vec![file("tax.pdf", 9 * GIB, 400)]),
                dir("tiny", vec![dir(".cache", vec![file("t", 1024, 1)])]),
            ],
        ))
    }

    #[test]
    fn ranks_findings_largest_first_and_skips_the_tiny() {
        let found = worth_a_look(home().root(), NOW, 10, Weighing::Allocated);
        let kinds: Vec<&Finding> = found.iter().map(|c| &c.finding).collect();
        assert_eq!(
            kinds,
            vec![
                &Finding::Reclaimable(Reclaim::Regenerable),
                &Finding::Worktrees {
                    count: 2,
                    oldest_days: 41
                },
                &Finding::StaleExperiments { count: 1 },
                &Finding::Reclaimable(Reclaim::BuildOutput),
            ]
        );
        assert_eq!(found[0].bytes, 5 * GIB);
        assert_eq!(found[2].bytes, 4 * GIB, "only the stale experiment counts");
    }

    #[test]
    fn documents_are_never_suggested() {
        let found = worth_a_look(home().root(), NOW, 10, Weighing::Allocated);
        assert!(found.iter().all(|c| c.crumbs != vec![3]));
    }

    #[test]
    fn crumbs_address_the_finding_from_the_root() {
        let tree = home();
        let root = tree.root();
        for candidate in worth_a_look(root, NOW, 10, Weighing::Allocated) {
            let node = root.resolve(&candidate.crumbs).expect("resolves");
            assert!(node.is_dir());
        }
    }

    #[test]
    fn the_limit_keeps_the_largest() {
        let found = worth_a_look(home().root(), NOW, 2, Weighing::Allocated);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[1].finding,
            Finding::Worktrees {
                count: 2,
                oldest_days: 41
            }
        );
    }

    /// Under Private a suggestion is chosen by what it holds alone: a
    /// worktrees folder of clones (150 MiB allocated, nothing alone) and a
    /// cache of clones are hidden, and a 70 MiB plain cache that ranks
    /// third by allocation takes their place. Allocated, the list is the
    /// clones again.
    #[test]
    fn private_weighing_hides_what_holds_too_little_alone() {
        let tree = scan(dir(
            "home",
            vec![
                dir(
                    ".codex",
                    vec![dir(
                        "worktrees",
                        vec![dir("a1", vec![clone("x", 150 * MIB, 0, 1)])],
                    )],
                ),
                dir(
                    "p",
                    vec![dir(".cache", vec![clone("y", 100 * MIB, 0, 2)])],
                ),
                dir("q", vec![dir(".cache", vec![file("z", 70 * MIB, 1)])]),
            ],
        ));
        let crumbs = |weighing| -> Vec<Vec<usize>> {
            worth_a_look(tree.root(), NOW, 2, weighing)
                .into_iter()
                .map(|candidate| candidate.crumbs)
                .collect()
        };
        assert_eq!(crumbs(Weighing::Allocated), vec![vec![0, 0], vec![1, 0]]);
        assert_eq!(crumbs(Weighing::Private), vec![vec![2, 0]]);
    }

    /// One `.cache` per top-level folder: its allocation in MiB, and the
    /// percentage of it the cache holds alone. 100 is a plain file.
    fn caches() -> impl Strategy<Value = Vec<(u64, u64)>> {
        prop::collection::vec(
            (
                1_u64..=200,
                prop_oneof![1 => Just(100_u64), 3 => 0_u64..=100],
            ),
            0..14,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// The list is the `limit` largest findings of at least
        /// `MIN_BYTES` as the weighing measures them, largest first,
        /// equal ones in the order they were found. The oracle filters,
        /// sorts and cuts the findings it built, by the figure the
        /// weighing names.
        #[test]
        fn the_weighing_chooses_and_ranks_the_list(
            caches in caches(),
            limit in 1_usize..=10,
            private in any::<bool>(),
        ) {
            let weighing = if private {
                Weighing::Private
            } else {
                Weighing::Allocated
            };
            // Each folder's name, allocation and what its cache holds alone.
            let specs: Vec<(String, u64, u64)> = caches
                .iter()
                .enumerate()
                .map(|(index, &(mib, percent))| {
                    let bytes = mib * MIB;
                    let alone = if percent == 100 {
                        bytes
                    } else {
                        bytes / 100 * percent
                    };
                    (format!("p{index}"), bytes, alone)
                })
                .collect();
            let folders = specs
                .iter()
                .zip(1_u64..)
                .map(|((name, bytes, alone), id)| {
                    let blob = if alone == bytes {
                        file("blob", *bytes, 1)
                    } else {
                        clone("blob", *bytes, *alone, id)
                    };
                    dir(name, vec![dir(".cache", vec![blob])])
                })
                .collect();
            let tree = scan(dir("home", folders));
            let root = tree.root();
            // The scan orders siblings its own way; equal findings keep
            // that order, so the oracle breaks ties by it.
            let place = |name: &str| {
                root.children()
                    .iter()
                    .position(|child| child.name() == name)
                    .expect("generated folder")
            };
            let mut oracle: Vec<(String, u64, u64)> = specs
                .into_iter()
                .filter(|&(_, bytes, alone)| {
                    (if private { alone } else { bytes }) >= MIN_BYTES
                })
                .collect();
            oracle.sort_by_key(|(name, bytes, alone)| {
                let weight = if private { *alone } else { *bytes };
                (std::cmp::Reverse(weight), place(name))
            });
            oracle.truncate(limit);
            let found: Vec<(String, u64, u64)> =
                worth_a_look(root, NOW, limit, weighing)
                    .into_iter()
                    .map(|c| {
                        let folder = root.resolve(&c.crumbs[..1]).unwrap();
                        assert_eq!(c.crumbs.len(), 2, "the folder's .cache");
                        (folder.name().to_owned(), c.bytes, c.private)
                    })
                    .collect();
            prop_assert_eq!(found, oracle);
        }
    }
}
