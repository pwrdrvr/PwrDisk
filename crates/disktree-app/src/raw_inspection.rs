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
    created: Instant,
    /// One result per planned mark, including the path in any refusal.
    result: Option<Vec<Result<(), String>>>,
}

impl RawInspection {
    fn matches(&self, app: &Disktree) -> bool {
        self.root == app.root_path
            && self.marks == app.marks.items()
            && self.created.elapsed() < LIFETIME
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
            disktree_core::cleanup::inspect_raw(&plan.targets)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(check) = &mut this.raw_inspection
                    && check.created == created
                {
                    check.result = Some(result);
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
        check
            .result
            .as_ref()
            .ok_or(PREPARING)?
            .iter()
            .cloned()
            .collect()
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
                    && check.result.as_ref().is_some_and(|results| {
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
