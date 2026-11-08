//! One saved display choice shared by the window and human-readable reports.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::size::{self, SizeUnits};

pub const FILE_NAME: &str = "size-units";

/// The same per-user directory used by the window's other preferences.
pub fn settings_path() -> Option<PathBuf> {
    let home = std::env::home_dir();
    let base = if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.map(|home| home.join(".config")))
    };
    base.map(|base| base.join("pwrdisk").join(FILE_NAME))
}

pub fn load(path: &Path) -> io::Result<SizeUnits> {
    match std::fs::read_to_string(path) {
        Ok(value) => SizeUnits::parse(value.trim()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "unknown size units")
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(SizeUnits::default())
        }
        Err(error) => Err(error),
    }
}

/// Atomic replacement preserves the last choice if writing fails.
pub fn save(path: &Path, units: SizeUnits) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "no settings directory")
    })?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    writeln!(file, "{}", units.key())?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Call before displaying sizes in an application process. Libraries keep
/// their historical binary formatting until the application opts in.
pub fn initialize() {
    let units = settings_path()
        .map_or_else(|| Ok(SizeUnits::default()), |path| load(&path))
        .unwrap_or_else(|error| {
            eprintln!(
                "Cannot load Size units: {error}; using Platform default"
            );
            SizeUnits::default()
        });
    size::set_units(units);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        // A restart must read the last successful pick, including a reset
        // to Platform default. Failed writes must not replace that pick.
        #[test]
        fn saved_choices_follow_the_last_successful_write(
            choices in prop::collection::vec((0_usize..3, any::<bool>()), 1..30)
        ) {
            let dir = tempfile::tempdir().expect("settings");
            let path = dir.path().join(FILE_NAME);
            let mut model = SizeUnits::default();
            prop_assert_eq!(load(&path).expect("missing"), model);
            for (index, fails) in choices {
                let picked = SizeUnits::ALL[index];
                if fails {
                    // A file cannot contain another settings file.
                    let blocker = dir.path().join("blocker");
                    std::fs::write(&blocker, "file").expect("blocker");
                    prop_assert!(save(&blocker.join(FILE_NAME), picked).is_err());
                } else {
                    save(&path, picked).expect("save");
                    model = picked;
                }
                prop_assert_eq!(load(&path).expect("restart"), model);
            }
        }
    }

    #[test]
    fn invalid_preferences_are_reported_without_rewriting_them() {
        let dir = tempfile::tempdir().expect("settings");
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "unknown").expect("bad setting");
        assert_eq!(
            load(&path).expect_err("invalid").kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            std::fs::read_to_string(path).expect("unchanged"),
            "unknown"
        );
    }
}
