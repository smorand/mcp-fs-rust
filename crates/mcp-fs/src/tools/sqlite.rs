//! SQLite tools: persistent SQLite databases stored as files in volumes.
//!
//! Every operation follows the extract-operate-commit pattern:
//!   1. Read the .db bytes from the volume blob store.
//!   2. Write them to a temporary file (rusqlite needs a real POSIX path).
//!   3. Open the SQLite connection on the temp file.
//!   4. Execute the query.
//!   5. For writes: read the modified bytes back and commit to the volume via
//!      fs_ops::write_bytes (quota + ACL + audit log enforced).
//!   6. The temp file is dropped (deleted) automatically.
//!
//! Concurrent access to the same (mount_id, db_path) is serialised by a
//! process-wide lock map so two requests cannot corrupt the same database.

use crate::config::SqliteConfig;
use crate::core::fs_ops;
use crate::errors::{Result, ToolError};
use crate::mcp::registry::handler;
use crate::mcp::{ToolRegistry, ToolSchema};
use crate::tools::{norm, volume};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

// Key: "{mount_id}:{db_path}" serialises concurrent access to the same database.
type DbLockMap = std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>;

fn db_lock_map() -> &'static DbLockMap {
    static MAP: OnceLock<DbLockMap> = OnceLock::new();
    MAP.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn db_lock(mount_id: &str, db_path: &str) -> Arc<Mutex<()>> {
    let key = format!("{mount_id}:{db_path}");
    let mut map = db_lock_map().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
}

/// Read db bytes from the volume and write them to a NamedTempFile.
/// Returns (tempfile, connection). The tempfile must stay alive for the connection's lifetime.
async fn open_db(
    client: &crate::storage::VolumeClient,
    db_path: &str,
) -> Result<(tempfile::NamedTempFile, Connection)> {
    let bytes = if client.exists(db_path).await? {
        client.read_bytes(db_path).await?
    } else {
        Vec::new()
    };

    let tmp = tempfile::NamedTempFile::new()
        .map_err(|e| ToolError::internal(format!("temp file: {e}")))?;
    std::fs::write(tmp.path(), &bytes)
        .map_err(|e| ToolError::internal(format!("write temp: {e}")))?;

    let conn = Connection::open(tmp.path())
        .map_err(|e| ToolError::internal(format!("open sqlite: {e}")))?;

    Ok((tmp, conn))
}

/// Read the (possibly modified) temp file bytes and write them into the volume.
async fn commit_db(
    tmp: &tempfile::NamedTempFile,
    client: &crate::storage::VolumeClient,
    safety: &crate::safety::SafetyManager,
    person: &str,
    mount_id: &str,
    db_path: &str,
) -> Result<()> {
    let bytes = std::fs::read(tmp.path())
        .map_err(|e| ToolError::internal(format!("read temp: {e}")))?;
    fs_ops::write_bytes(client, safety, person, mount_id, db_path, &bytes, true, true).await?;
    Ok(())
}

fn cap_rows(requested: i64, config_max: usize) -> usize {
    const HARD_CAP: usize = 10_000;
    let max = config_max.min(HARD_CAP);
    if requested <= 0 { max } else { (requested as usize).min(max) }
}

/// Returns Err if `sql` is not a SELECT or WITH statement.
fn require_select(sql: &str) -> Result<()> {
    let first = sql.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    if first == "SELECT" || first == "WITH" {
        Ok(())
    } else {
        Err(ToolError::invalid_argument(
            "sqlite.query only accepts SELECT or WITH statements; use sqlite.execute for writes",
        ))
    }
}

/// Execute a SELECT and collect results as {columns, rows, row_count}.
fn select_json(conn: &Connection, sql: &str, raw_params: &[Value], limit: usize) -> Result<Value> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| ToolError::invalid_argument(format!("SQL error: {e}")))?;

    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let sql_params: Vec<Box<dyn rusqlite::ToSql>> = raw_params
        .iter()
        .map(|v| -> Box<dyn rusqlite::ToSql> {
            match v {
                Value::Null => Box::new(rusqlite::types::Null),
                Value::Bool(b) => Box::new(*b as i64),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Box::new(i)
                    } else {
                        Box::new(n.as_f64().unwrap_or(0.0))
                    }
                }
                Value::String(s) => Box::new(s.clone()),
                other => Box::new(other.to_string()),
            }
        })
        .collect();

    let refs: Vec<&dyn rusqlite::ToSql> = sql_params.iter().map(|b| b.as_ref()).collect();

    let mut rows_out: Vec<Value> = Vec::new();
    let mut raw_rows = stmt
        .query(refs.as_slice())
        .map_err(|e| ToolError::invalid_argument(format!("query error: {e}")))?;

    while let Some(row) = raw_rows.next().map_err(|e| ToolError::internal(format!("row: {e}")))? {
        if rows_out.len() >= limit {
            break;
        }
        let obj: serde_json::Map<String, Value> = col_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let v: Value = match row.get_ref(i) {
                    Ok(rusqlite::types::ValueRef::Null) => Value::Null,
                    Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
                    Ok(rusqlite::types::ValueRef::Real(f)) => json!(f),
                    Ok(rusqlite::types::ValueRef::Text(s)) => {
                        Value::String(String::from_utf8_lossy(s).into_owned())
                    }
                    Ok(rusqlite::types::ValueRef::Blob(b)) => {
                        Value::String(format!("<blob {} bytes>", b.len()))
                    }
                    Err(_) => Value::Null,
                };
                (name.clone(), v)
            })
            .collect();
        rows_out.push(Value::Object(obj));
    }

    let row_count = rows_out.len();
    Ok(json!({
        "columns": col_names,
        "rows": rows_out,
        "row_count": row_count,
    }))
}

/// Parse CSV bytes into (headers, rows). Minimal implementation: splits on
/// delimiter, trims whitespace, skips empty lines. Caps at 100 000 rows.
pub fn parse_csv(bytes: &[u8], delimiter: char) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());

    let header_line = lines.next().ok_or_else(|| ToolError::invalid_argument("CSV has no header row"))?;
    let headers: Vec<String> = header_line.split(delimiter).map(|s| s.trim().to_string()).collect();

    const ROW_CAP: usize = 100_000;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        if rows.len() >= ROW_CAP {
            break;
        }
        let row: Vec<String> = line.split(delimiter).map(|s| s.trim().to_string()).collect();
        rows.push(row);
    }
    Ok((headers, rows))
}

pub fn register(reg: &mut ToolRegistry, config: &SqliteConfig) {
    let max_rows = config.max_result_rows;

    // sqlite.query
    reg.add(
        ToolSchema::new("sqlite.query", "Execute a SELECT query against a SQLite database stored in a volume. Returns columns, rows, and row_count.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the .db file within the volume.")
            .req_str("sql", "SELECT or WITH statement to execute.")
            .opt_str_null("params", "JSON array of bind parameters.")
            .opt_int("max_rows", 100, "Maximum rows to return (capped at config limit)."),
        handler(move |ctx, a| async move {
            let sql = a.str("sql")?;
            require_select(&sql)?;
            let limit = cap_rows(a.int_or("max_rows", 100), max_rows);
            let raw_params = parse_params(a.opt_str("params").as_deref())?;
            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (_tmp, conn) = open_db(&client, &db_path).await?;
            select_json(&conn, &sql, &raw_params, limit)
        }),
    );

    // sqlite.execute
    reg.add(
        ToolSchema::new("sqlite.execute", "Execute a write statement (INSERT, UPDATE, DELETE, CREATE, DROP, etc.) against a SQLite database stored in a volume. Returns rows_affected and last_insert_rowid.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the .db file within the volume.")
            .req_str("sql", "SQL statement to execute.")
            .opt_str_null("params", "JSON array of bind parameters."),
        handler(move |ctx, a| async move {
            let sql = a.str("sql")?;
            let raw_params = parse_params(a.opt_str("params").as_deref())?;
            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (tmp, conn) = open_db(&client, &db_path).await?;

            let (rows_affected, last_insert_rowid) = {
                let sql_params: Vec<Box<dyn rusqlite::ToSql>> = raw_params
                    .iter()
                    .map(|v| -> Box<dyn rusqlite::ToSql> {
                        match v {
                            Value::Null => Box::new(rusqlite::types::Null),
                            Value::Bool(b) => Box::new(*b as i64),
                            Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    Box::new(i)
                                } else {
                                    Box::new(n.as_f64().unwrap_or(0.0))
                                }
                            }
                            Value::String(s) => Box::new(s.clone()),
                            other => Box::new(other.to_string()),
                        }
                    })
                    .collect();
                let refs: Vec<&dyn rusqlite::ToSql> =
                    sql_params.iter().map(|b| b.as_ref()).collect();
                let ra = conn
                    .execute(&sql, refs.as_slice())
                    .map_err(|e| ToolError::invalid_argument(format!("SQL error: {e}")))?;
                let lirid = conn.last_insert_rowid();
                (ra, lirid)
                // refs and sql_params drop here, before any await
            };

            drop(conn);
            commit_db(&tmp, &client, &ctx.state.safety, &ctx.person, &mount, &db_path).await?;

            Ok(json!({ "rows_affected": rows_affected, "last_insert_rowid": last_insert_rowid }))
        }),
    );

    // sqlite.list_tables
    reg.add(
        ToolSchema::new("sqlite.list_tables", "List tables and views in a SQLite database stored in a volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the .db file within the volume."),
        handler(move |ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (_tmp, conn) = open_db(&client, &db_path).await?;

            let result = select_json(
                &conn,
                "SELECT name, type FROM sqlite_master WHERE type IN ('table','view') ORDER BY name",
                &[],
                10_000,
            )?;
            Ok(result["rows"].clone())
        }),
    );

    // sqlite.describe_table
    reg.add(
        ToolSchema::new("sqlite.describe_table", "Describe the columns and indexes of a table in a SQLite database stored in a volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the .db file within the volume.")
            .req_str("table", "Table name to inspect."),
        handler(move |ctx, a| async move {
            let table = a.str("table")?;
            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (_tmp, conn) = open_db(&client, &db_path).await?;

            let cols_sql = format!("PRAGMA table_info({table})");
            let columns = select_json(&conn, &cols_sql, &[], 10_000)?;

            let idx_sql = format!("PRAGMA index_list({table})");
            let indexes = select_json(&conn, &idx_sql, &[], 10_000)?;

            Ok(json!({ "columns": columns["rows"], "indexes": indexes["rows"] }))
        }),
    );

    // sqlite.list_indexes
    reg.add(
        ToolSchema::new("sqlite.list_indexes", "List indexes in a SQLite database stored in a volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the .db file within the volume.")
            .opt_str_null("table", "Filter to a specific table; omit for all indexes."),
        handler(move |ctx, a| async move {
            let table = a.opt_str("table");
            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (_tmp, conn) = open_db(&client, &db_path).await?;

            let result = if let Some(t) = table {
                let sql = format!("PRAGMA index_list({t})");
                select_json(&conn, &sql, &[], 10_000)?
            } else {
                select_json(
                    &conn,
                    "SELECT name, tbl_name FROM sqlite_master WHERE type='index' ORDER BY tbl_name, name",
                    &[],
                    10_000,
                )?
            };
            Ok(result["rows"].clone())
        }),
    );

    // sqlite.vacuum
    reg.add(
        ToolSchema::new("sqlite.vacuum", "Run VACUUM on a SQLite database stored in a volume to reclaim space.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the .db file within the volume."),
        handler(move |ctx, a| async move {
            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (tmp, conn) = open_db(&client, &db_path).await?;

            conn.execute_batch("VACUUM")
                .map_err(|e| ToolError::internal(format!("VACUUM failed: {e}")))?;
            drop(conn);
            commit_db(&tmp, &client, &ctx.state.safety, &ctx.person, &mount, &db_path).await?;

            Ok(json!({ "compacted": true, "path": db_path }))
        }),
    );

    // sqlite.import_csv
    reg.add(
        ToolSchema::new("sqlite.import_csv", "Import a CSV file from the volume into a SQLite database table.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the target .db file within the volume.")
            .req_str("csv_path", "Source CSV file path in the volume.")
            .req_str("table", "Target table name.")
            .opt_bool("create_table", true, "Create the table if it does not exist (infers schema from CSV header).")
            .opt_str(",", ",", "Column delimiter character."),
        handler(move |ctx, a| async move {
            let csv_path_raw = a.str("csv_path")?;
            let table = a.str("table")?;
            let create_table = a.bool_or("create_table", true);
            let delim_str = a.str_or(",", ",");
            let delimiter = delim_str.chars().next().unwrap_or(',');

            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let csv_path = ctx.state.safety.normalize_path(&csv_path_raw)?;

            let csv_bytes = client.read_bytes(&csv_path).await?;
            let (headers, rows) = parse_csv(&csv_bytes, delimiter)?;
            let rows_imported = rows.len();

            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (tmp, conn) = open_db(&client, &db_path).await?;

            if create_table {
                let cols: Vec<String> = headers.iter().map(|h| format!("{h} TEXT")).collect();
                let create_sql =
                    format!("CREATE TABLE IF NOT EXISTS \"{table}\" ({})", cols.join(", "));
                conn.execute_batch(&create_sql)
                    .map_err(|e| ToolError::invalid_argument(format!("CREATE TABLE failed: {e}")))?;
            }

            let placeholders: Vec<&str> = headers.iter().map(|_| "?").collect();
            let col_names: Vec<String> = headers.iter().map(|h| format!("\"{h}\"")).collect();
            let insert_sql = format!(
                "INSERT INTO \"{table}\" ({}) VALUES ({})",
                col_names.join(", "),
                placeholders.join(", ")
            );

            {
                let tx = conn
                    .unchecked_transaction()
                    .map_err(|e| ToolError::internal(format!("transaction: {e}")))?;
                let mut stmt = tx
                    .prepare(&insert_sql)
                    .map_err(|e| ToolError::invalid_argument(format!("prepare insert: {e}")))?;

                for row in &rows {
                    let params: Vec<&str> = headers
                        .iter()
                        .enumerate()
                        .map(|(i, _)| row.get(i).map(|s| s.as_str()).unwrap_or(""))
                        .collect();
                    stmt.execute(rusqlite::params_from_iter(params.iter()))
                        .map_err(|e| ToolError::internal(format!("insert row: {e}")))?;
                }
                drop(stmt);
                tx.commit()
                    .map_err(|e| ToolError::internal(format!("commit: {e}")))?;
            }

            drop(conn);
            commit_db(&tmp, &client, &ctx.state.safety, &ctx.person, &mount, &db_path).await?;

            Ok(json!({ "rows_imported": rows_imported, "table": table }))
        }),
    );

    // sqlite.export_csv
    reg.add(
        ToolSchema::new("sqlite.export_csv", "Export a SELECT query result from a SQLite database in a volume to a CSV file in the same volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("db_path", "Path to the source .db file within the volume.")
            .req_str("sql", "SELECT statement whose results will be exported.")
            .req_str("output_path", "Destination CSV path in the volume.")
            .opt_str(",", ",", "Column delimiter character."),
        handler(move |ctx, a| async move {
            let sql = a.str("sql")?;
            require_select(&sql)?;
            let output_path_raw = a.str("output_path")?;
            let delim_str = a.str_or(",", ",");
            let delimiter = delim_str.chars().next().unwrap_or(',');

            let (mount, client) = volume(&ctx, &a).await?;
            let db_path = norm(&ctx, &a, "db_path")?;
            let output_path = ctx.state.safety.normalize_path(&output_path_raw)?;

            let lock = db_lock(&mount, &db_path);
            let _guard = lock.lock().await;
            let (_tmp, conn) = open_db(&client, &db_path).await?;

            let result = select_json(&conn, &sql, &[], 10_000)?;
            let cols = result["columns"].as_array().unwrap();
            let rows = result["rows"].as_array().unwrap();
            let rows_exported = rows.len();

            let mut csv = String::new();
            let header_line: Vec<String> = cols
                .iter()
                .map(|c| csv_escape(c.as_str().unwrap_or(""), delimiter))
                .collect();
            csv.push_str(&header_line.join(&delimiter.to_string()));
            csv.push('\n');

            for row in rows {
                let obj = row.as_object().unwrap();
                let line: Vec<String> = cols
                    .iter()
                    .map(|c| {
                        let key = c.as_str().unwrap_or("");
                        let val = obj.get(key).map(value_to_csv).unwrap_or_default();
                        csv_escape(&val, delimiter)
                    })
                    .collect();
                csv.push_str(&line.join(&delimiter.to_string()));
                csv.push('\n');
            }

            fs_ops::write_bytes(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &output_path,
                csv.as_bytes(),
                true,
                true,
            )
            .await?;

            Ok(json!({ "rows_exported": rows_exported, "path": output_path }))
        }),
    );
}

fn parse_params(raw: Option<&str>) -> Result<Vec<Value>> {
    match raw {
        None => Ok(Vec::new()),
        Some(s) => {
            let v: Value = serde_json::from_str(s)
                .map_err(|e| ToolError::invalid_argument(format!("params must be a JSON array: {e}")))?;
            match v {
                Value::Array(arr) => Ok(arr),
                _ => Err(ToolError::invalid_argument("params must be a JSON array")),
            }
        }
    }
}

fn value_to_csv(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn csv_escape(s: &str, delimiter: char) -> String {
    if s.contains('"') || s.contains('\n') || s.contains(delimiter) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SqliteConfig;
    use crate::mcp::ToolRegistry;

    fn reg() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r, &SqliteConfig::default());
        r
    }

    #[test]
    fn eight_sqlite_tools_register() {
        assert_eq!(reg().len(), 8);
        assert_eq!(
            reg().names(),
            [
                "sqlite.query",
                "sqlite.execute",
                "sqlite.list_tables",
                "sqlite.describe_table",
                "sqlite.list_indexes",
                "sqlite.vacuum",
                "sqlite.import_csv",
                "sqlite.export_csv",
            ]
        );
    }

    #[test]
    fn select_only_enforcement() {
        assert!(require_select("SELECT 1").is_ok());
        assert!(require_select("  select * from t").is_ok());
        assert!(require_select("WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
        assert!(require_select("INSERT INTO t VALUES (1)").is_err());
        assert!(require_select("UPDATE t SET x=1").is_err());
        assert!(require_select("DROP TABLE t").is_err());
    }

    #[test]
    fn cap_rows_enforces_hard_cap() {
        let cfg_max = 1_000;
        assert_eq!(cap_rows(0, cfg_max), 1_000);
        assert_eq!(cap_rows(50, cfg_max), 50);
        assert_eq!(cap_rows(5_000, cfg_max), 1_000);
        assert_eq!(cap_rows(20_000, 10_000), 10_000);
    }

    #[test]
    fn parse_csv_splits_header_and_rows() {
        let data = b"name,age\nAlice,30\nBob,25\n";
        let (headers, rows) = parse_csv(data, ',').unwrap();
        assert_eq!(headers, ["name", "age"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ["Alice", "30"]);
    }

    #[tokio::test]
    async fn create_and_query_in_memory_roundtrip() {
        let h = crate::tools::testkit::harness_with_extra(|reg, cfg| {
            register(reg, &cfg.sqlite);
        })
        .await;

        let result = h
            .call(
                "sqlite.execute",
                serde_json::json!({
                    "mount_id": crate::tools::testkit::MOUNT,
                    "db_path": "/test.db",
                    "sql": "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["rows_affected"], 0);

        h.call(
            "sqlite.execute",
            serde_json::json!({
                "mount_id": crate::tools::testkit::MOUNT,
                "db_path": "/test.db",
                "sql": "INSERT INTO items (name) VALUES ('hello'), ('world')"
            }),
        )
        .await
        .unwrap();

        let result = h
            .call(
                "sqlite.query",
                serde_json::json!({
                    "mount_id": crate::tools::testkit::MOUNT,
                    "db_path": "/test.db",
                    "sql": "SELECT * FROM items ORDER BY id"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["row_count"], 2);
        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows[0]["name"], "hello");
        assert_eq!(rows[1]["name"], "world");
    }

    #[tokio::test]
    async fn list_tables_after_create() {
        let h = crate::tools::testkit::harness_with_extra(|reg, cfg| {
            register(reg, &cfg.sqlite);
        })
        .await;
        h.call(
            "sqlite.execute",
            serde_json::json!({
                "mount_id": crate::tools::testkit::MOUNT,
                "db_path": "/t.db",
                "sql": "CREATE TABLE foo (x INT)"
            }),
        )
        .await
        .unwrap();
        h.call(
            "sqlite.execute",
            serde_json::json!({
                "mount_id": crate::tools::testkit::MOUNT,
                "db_path": "/t.db",
                "sql": "CREATE TABLE bar (y TEXT)"
            }),
        )
        .await
        .unwrap();
        let result = h
            .call(
                "sqlite.list_tables",
                serde_json::json!({
                    "mount_id": crate::tools::testkit::MOUNT,
                    "db_path": "/t.db"
                }),
            )
            .await
            .unwrap();
        let tables = result.as_array().unwrap();
        assert!(tables.iter().any(|t| t["name"] == "foo"));
        assert!(tables.iter().any(|t| t["name"] == "bar"));
    }
}
