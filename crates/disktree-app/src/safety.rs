//! User-configured removal limit; MCP cannot change it.

use std::path::Path;

use disktree_core::removal_limit::Limit;

const FILE: &str = "removal-limit-percent";

pub use disktree_core::removal_limit::load_settings as load;

pub fn save(dir: Option<&Path>, limit: Limit) -> Result<(), String> {
    let dir = dir.ok_or("No settings directory is available")?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let pending = dir.join("removal-limit-percent.new");
    std::fs::write(&pending, format!("{}\n", limit.percent()))
        .map_err(|error| error.to_string())?;
    std::fs::rename(pending, dir.join(FILE)).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_persist_and_invalid_configuration_refuses_removal() {
        let temp = tempfile::tempdir().expect("settings");
        let dir = Some(temp.path());
        assert_eq!(load(dir).expect("default").percent(), 5);
        save(dir, Limit::new(10).expect("10%")).expect("save");
        assert_eq!(load(dir).expect("saved").percent(), 10);
        std::fs::write(temp.path().join(FILE), "0").expect("invalid setting");
        assert!(load(dir).is_err());
    }
}
