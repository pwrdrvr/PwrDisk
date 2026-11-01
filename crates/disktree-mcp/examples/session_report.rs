//! Print what `pwrdisk mcp` answers for a directory, without a window.
//!
//! `cargo run -p disktree-mcp --example session_report -- DIR [MARK...]`
//! scans DIR here (no socket, no window is launched), then prints the
//! scan report, `pwrdisk_ls DIR`, `pwrdisk_find DIR` and, for each MARK
//! argument (a comma-separated list of paths), the mark plan for that
//! set. Useful for checking how a fixture reads to an agent.

use std::path::PathBuf;

use disktree_mcp::Session;
use disktree_mcp::session::{
    FindParams, LsParams, PathsParams, ScanParams, ScanPlan, run_scan,
};

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("usage: DIR [MARK,...]")?);
    // A socket nobody listens on, and no launcher: the scan runs here.
    let socket = std::env::temp_dir().join("pwrdisk-session-report.sock");
    let mut session = Session::detached(socket, None);
    let ScanPlan::Local { root, options } =
        session.prepare_scan(&ScanParams {
            path: Some(root),
            ..ScanParams::default()
        })?
    else {
        return Err("a window answered; this example scans here".into());
    };
    let scan = run_scan(root.clone(), options)?;
    println!("== pwrdisk_scan\n{}\n", session.store_scan(scan));
    let ls = LsParams {
        path: root.clone(),
        ..LsParams::default()
    };
    println!("== pwrdisk_ls\n{}", session.ls(&ls)?);
    let find = FindParams {
        path: Some(root),
        ..FindParams::default()
    };
    println!("== pwrdisk_find\n{}", session.find(&find)?);
    for set in args {
        let paths: Vec<PathBuf> = set
            .to_string_lossy()
            .split(',')
            .map(PathBuf::from)
            .collect();
        let names = set.to_string_lossy().into_owned();
        let params = PathsParams { paths };
        println!("== pwrdisk_mark {names}\n{}", session.mark(&params)?);
        session.unmark(&PathsParams::default())?;
    }
    Ok(())
}
