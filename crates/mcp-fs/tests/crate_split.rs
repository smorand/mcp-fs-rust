//! DT-010: after the DBT-002/DR-002 crate split, `crates/mcp-fs/src/` contains
//! exactly `main.rs` and no logic modules (SPEC-0012).

use std::fs;
use std::path::Path;

#[test]
fn mcp_fs_src_contains_only_main_rs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut entries: Vec<String> = fs::read_dir(&src)
        .expect("read crates/mcp-fs/src")
        .map(|e| e.expect("dir entry").file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["main.rs".to_string()]);
}
