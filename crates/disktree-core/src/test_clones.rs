//! Random trees of plain files and APFS clones, for properties that must
//! hold of private sizes whatever the tree's shape.

use proptest::prelude::*;

use crate::sharing::{Allocation, CloneInfo};
use crate::tree::{Draft, Metric, NodeKind, Tree, aggregate};

/// One generated file: its size, and its clone facts if it is one.
#[derive(Clone, Debug)]
pub struct FileSpec {
    pub bytes: u64,
    pub clone: Cloned,
}

/// Whether a generated file is a clone, and what the scan read of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cloned {
    Plain,
    /// A clone whose private size the scan could not read.
    Unknown,
    /// A clone with this much private, which may exceed its size, as a
    /// scan that raced a write can see.
    Private(u64),
}

impl FileSpec {
    /// What this file holds alone, worked out on its own: the oracle the
    /// tree's figure is checked against.
    pub fn private(&self) -> u64 {
        match self.clone {
            Cloned::Plain => self.bytes,
            Cloned::Unknown => 0,
            Cloned::Private(private) => private.min(self.bytes),
        }
    }

    fn draft(&self, name: String) -> Draft {
        let mut file = Draft::entry(name, NodeKind::File, self.bytes);
        let private = match self.clone {
            Cloned::Plain => return file,
            Cloned::Unknown => None,
            Cloned::Private(private) => Some(private),
        };
        file.allocation = Some(Box::new(Allocation {
            bytes: self.bytes,
            identity: None,
            links: 1,
            followed: false,
            clone: Some(CloneInfo {
                device: 1,
                // A few ids, so some files share a group.
                id: 1 + self.bytes % 3,
                references: 2,
                private_bytes: private,
                data_bytes: self.bytes,
            }),
        }));
        file
    }
}

/// A directory: its files, then its subdirectories.
#[derive(Clone, Debug)]
pub struct DirSpec {
    pub files: Vec<FileSpec>,
    pub dirs: Vec<Self>,
}

impl DirSpec {
    /// Every file at or beneath this directory.
    pub fn all_files(&self) -> Vec<&FileSpec> {
        let mut files: Vec<&FileSpec> = self.files.iter().collect();
        for dir in &self.dirs {
            files.extend(dir.all_files());
        }
        files
    }

    fn draft(&self, name: String) -> Draft {
        let mut dir = Draft::directory(name);
        for (index, file) in self.files.iter().enumerate() {
            dir.children.push(file.draft(format!("f{index}")));
        }
        for (index, sub) in self.dirs.iter().enumerate() {
            dir.children.push(sub.draft(format!("d{index}")));
        }
        dir
    }

    /// The tree a scan would build of this, totals and sharing settled.
    pub fn tree(&self) -> Tree {
        let mut tree = Tree::from(self.draft("root".into()));
        aggregate(&mut tree, Metric::Bytes);
        tree
    }
}

pub fn file() -> impl Strategy<Value = FileSpec> {
    (
        0_u64..10_000,
        prop_oneof![
            2 => Just(Cloned::Plain),
            1 => Just(Cloned::Unknown),
            3 => (0_u64..12_000).prop_map(Cloned::Private),
        ],
    )
        .prop_map(|(bytes, clone)| FileSpec { bytes, clone })
}

/// Up to three levels of directories under the root.
pub fn dir() -> impl Strategy<Value = DirSpec> {
    let leaf = prop::collection::vec(file(), 0..5).prop_map(|files| DirSpec {
        files,
        dirs: Vec::new(),
    });
    leaf.prop_recursive(3, 40, 4, |inner| {
        (
            prop::collection::vec(file(), 0..5),
            prop::collection::vec(inner, 0..4),
        )
            .prop_map(|(files, dirs)| DirSpec { files, dirs })
    })
}
