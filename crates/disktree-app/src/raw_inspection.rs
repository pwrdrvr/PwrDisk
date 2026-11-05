//! Review and agent previews share the worker's bounded cache inspection.
//! Results belong to an exact scan and selection; painting only reads them.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use disktree_core::removal::{RemovalMode, Target};
use disktree_core::tree::Tree;
use gpui_kit::Context;
use gpui_omarchy::Status;

use crate::state::{Disktree, Screen};

const LIFETIME: Duration = Duration::from_mins(5);
const PREPARING: &str = "Cache inspection is preparing; wait for it to finish before confirming removal";

#[derive(Debug)]
pub struct RawInspection {
    root: std::path::PathBuf,
    marks: Vec<Target>,
    tree: Weak<Tree>,
    /// Identifies the request; a pending walk never expires for age alone.
    created: Instant,
    /// Completion time and one result per planned mark, with refusal paths.
    result: Option<(Instant, Vec<Result<(), String>>)>,
}

impl RawInspection {
    fn fresh_at(&self, now: Instant) -> bool {
        self.result.as_ref().is_none_or(|(completed, _)| {
            now.duration_since(*completed) < LIFETIME
        })
    }

    fn complete(&mut self, result: Vec<Result<(), String>>, now: Instant) {
        self.result = Some((now, result));
    }

    fn matches(&self, app: &Disktree) -> bool {
        self.root == app.root_path
            && self.marks == app.marks.items()
            && self.fresh_at(Instant::now())
            && app.remote.is_none()
            && app.scan.is_none()
            && app.tree.as_ref().is_some_and(|tree| {
                Weak::ptr_eq(&self.tree, &Arc::downgrade(tree))
            })
    }
}

impl Disktree {
    pub(crate) fn prepare_raw_inspection(&mut self, cx: &Context<'_, Self>) {
        if self.remote.is_some()
            || self.scan.is_some()
            || self.screen == Screen::Running
            || self.restarting
            || self.marks.is_empty()
        {
            return;
        }
        let Some(tree) = &self.tree else {
            return;
        };
        if self
            .raw_inspection
            .as_ref()
            .is_some_and(|check| check.matches(self))
        {
            return;
        }
        let plan =
            disktree_core::removal::plan(self.marks.items(), &self.root_path);
        let created = Instant::now();
        self.raw_inspection = Some(RawInspection {
            root: self.root_path.clone(),
            marks: self.marks.items().to_vec(),
            tree: Arc::downgrade(tree),
            created,
            result: None,
        });
        let task = cx.background_executor().spawn(async move {
            let result = disktree_core::cleanup::inspect_raw(&plan.targets);
            (result, Instant::now())
        });
        cx.spawn(async move |this, cx| {
            let (result, completed) = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(check) = &mut this.raw_inspection
                    && check.created == created
                {
                    check.complete(result, completed);
                    this.route_inspected_caches(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn raw_inspection_ready(&self) -> Result<(), String> {
        let check = self
            .raw_inspection
            .as_ref()
            .filter(|check| check.matches(self))
            .ok_or(PREPARING)?;
        let (_, results) = check.result.as_ref().ok_or(PREPARING)?;
        results.iter().cloned().collect()
    }

    /// A pending check is visible too, so disabled confirmation explains why.
    pub(crate) fn raw_inspection_hint(&self) -> Option<String> {
        self.raw_inspection_ready().err()
    }

    pub(crate) fn route_inspected_caches(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        if self.screen == Screen::Review
            && self.raw_inspection.as_ref().is_some_and(|check| {
                check.matches(self)
                    && check.result.as_ref().is_some_and(|(_, results)| {
                        results.iter().any(Result::is_err)
                    })
            })
        {
            self.removal_mode = RemovalMode::OwnerCleanup;
            self.confirm_open = false;
            cx.notify();
        }
    }

    /// Both confirmation and the native removal entry point must wait.
    pub(crate) fn raw_removal_reviewed(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.prepare_raw_inspection(cx);
        match self.raw_inspection_ready() {
            Ok(()) => true,
            Err(reason) => {
                self.route_inspected_caches(cx);
                self.say(reason, Status::Warning, cx);
                cx.notify();
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn pending(created: Instant) -> RawInspection {
        RawInspection {
            root: std::path::PathBuf::new(),
            marks: Vec::new(),
            tree: Weak::new(),
            created,
            result: None,
        }
    }

    /// A walk's duration cannot evict its still-pending request.
    #[test]
    fn pending_raw_inspection_survives_its_result_lifetime() {
        let start = Instant::now();
        let check = pending(start);
        assert!(check.fresh_at(start + LIFETIME));
    }

    /// Finishing a slow walk grants a full lifetime to its new result.
    #[test]
    fn slow_raw_inspection_is_fresh_when_it_finishes() {
        let start = Instant::now();
        let mut check = pending(start);
        let finished = start + LIFETIME * 2;
        check.complete(vec![Ok(())], finished);
        assert!(check.fresh_at(finished));
        assert!(!check.fresh_at(finished + LIFETIME));
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Reference model: pending never ages out; ready expires exactly
        /// one lifetime after completion, independent of queue/walk time.
        #[test]
        fn raw_inspection_freshness_follows_completion_model(
            walk_seconds in 0_u64..3_600,
            idle_seconds in 0_u64..900,
            refused in any::<bool>(),
        ) {
            let start = Instant::now();
            let mut check = pending(start);
            let finished = start + Duration::from_secs(walk_seconds);
            prop_assert!(check.fresh_at(finished));
            let result = if refused { Err("cache".into()) } else { Ok(()) };
            check.complete(vec![result], finished);
            let idle = Duration::from_secs(idle_seconds);
            prop_assert_eq!(check.fresh_at(finished + idle), idle < LIFETIME);
        }
    }
}
