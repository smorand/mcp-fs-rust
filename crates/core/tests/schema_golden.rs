//! SPEC-0051/E2E-016: the stored layout is unchanged.
//!
//! The table definitions the four relational stores (meta, admin, git, oauth) render on
//! SQLite and on PostgreSQL are compared byte for byte with a golden captured BEFORE the
//! SQL Server removal. The removal touches the dialect and the column types, so only a
//! capture from the previous code can prove an existing database keeps opening.
//!
//! Regenerate deliberately with `MCPFS_REWRITE_SCHEMA_GOLDEN=1 cargo test -p mcp-fs-core
//! --test schema_golden` and review the diff: a changed golden is a layout change.

use mcp_fs_core::git::{db as git_db, oauth::persistence as oauth_db};
use mcp_fs_core::storage::rel::{Dialect, SchemaSet};
use mcp_fs_core::storage::{admin, meta};
use std::path::PathBuf;

fn render(dialect: Dialect) -> String {
    let stores: [(&str, SchemaSet); 4] = [
        ("meta", meta::schema()),
        ("admin", admin::schema()),
        ("git", git_db::schema()),
        ("oauth", oauth_db::schema()),
    ];
    let mut out = String::new();
    for (name, set) in stores {
        out.push_str(&format!("-- store: {name}\n"));
        for statement in set.render(dialect) {
            out.push_str(&statement);
            out.push_str("\n\n");
        }
    }
    out
}

fn golden_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(file)
}

fn check(dialect: Dialect, file: &str) {
    let actual = render(dialect);
    let path = golden_path(file);
    if std::env::var_os("MCPFS_REWRITE_SCHEMA_GOLDEN").is_some() {
        std::fs::write(&path, &actual).expect("write the schema golden");
        return;
    }
    let golden = std::fs::read_to_string(&path).expect("the committed schema golden");
    assert_eq!(actual, golden, "{file}: the stored layout changed");
}

#[test]
fn spec_0051_sqlite_table_definitions_are_byte_identical_to_the_golden() {
    check(Dialect::Sqlite, "schema_sqlite.sql");
}

#[test]
fn spec_0051_postgres_table_definitions_are_byte_identical_to_the_golden() {
    check(Dialect::Postgres, "schema_postgres.sql");
}
