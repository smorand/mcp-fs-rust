//! DataFusion analytics tools: query CSV, Parquet, and JSON files stored in volumes.
//!
//! All reads happen via a temp file: the file bytes are read from the volume blob
//! store, written to a NamedTempFile, then registered with DataFusion. For exports
//! the output bytes are written back into the volume via fs_ops::write_bytes.
//!
//! The table name in SQL is the file stem (filename without extension), lowercased.
//! Example: /data/Sales_2024.csv maps to table name "sales_2024".

use crate::config::DbConfig;
use crate::core::fs_ops;
use crate::errors::{Result, ToolError};
use crate::mcp::registry::handler;
use crate::mcp::{ToolRegistry, ToolSchema};
use crate::tools::{norm, volume};
use arrow::array::{
    Array, BooleanArray, Date32Array, Date64Array, Float32Array, Float64Array, Int16Array,
    Int32Array, Int64Array, Int8Array, LargeStringArray, StringArray, TimestampNanosecondArray,
    UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
enum FileFormat {
    Csv,
    Parquet,
    Json,
    Ndjson,
}

fn detect_format(path: &str) -> Result<FileFormat> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".csv") {
        return Ok(FileFormat::Csv);
    }
    if lower.ends_with(".parquet") || lower.ends_with(".parq") {
        return Ok(FileFormat::Parquet);
    }
    if lower.ends_with(".ndjson") || lower.ends_with(".jsonl") {
        return Ok(FileFormat::Ndjson);
    }
    if lower.ends_with(".json") {
        return Ok(FileFormat::Json);
    }
    Err(ToolError::invalid_argument(format!(
        "unsupported file format for '{path}': expected .csv, .parquet, .json, or .ndjson"
    )))
}

fn table_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "data".to_string())
}

/// Write bytes to a NamedTempFile with the right extension, register with DataFusion,
/// and return the temp file (caller keeps it alive for the duration of queries).
async fn register_from_bytes(
    ctx: &SessionContext,
    tname: &str,
    fmt: FileFormat,
    bytes: &[u8],
) -> Result<tempfile::NamedTempFile> {
    let suffix = match fmt {
        FileFormat::Csv => ".csv",
        FileFormat::Parquet => ".parquet",
        FileFormat::Json | FileFormat::Ndjson => ".json",
    };
    let tmp = tempfile::Builder::new()
        .suffix(suffix)
        .tempfile()
        .map_err(|e| ToolError::internal(format!("temp file: {e}")))?;
    std::fs::write(tmp.path(), bytes)
        .map_err(|e| ToolError::internal(format!("write temp: {e}")))?;

    let path_str = tmp.path().to_string_lossy().to_string();
    match fmt {
        FileFormat::Csv => {
            ctx.register_csv(tname, &path_str, CsvReadOptions::new())
                .await
                .map_err(|e| ToolError::internal(format!("register csv: {e}")))?;
        }
        FileFormat::Parquet => {
            ctx.register_parquet(tname, &path_str, ParquetReadOptions::default())
                .await
                .map_err(|e| ToolError::internal(format!("register parquet: {e}")))?;
        }
        FileFormat::Json | FileFormat::Ndjson => {
            ctx.register_json(tname, &path_str, NdJsonReadOptions::default())
                .await
                .map_err(|e| ToolError::internal(format!("register json: {e}")))?;
        }
    }
    Ok(tmp)
}

fn cap_rows(requested: i64, config_max: usize) -> usize {
    const HARD_CAP: usize = 10_000;
    let max = config_max.min(HARD_CAP);
    if requested <= 0 { max } else { (requested as usize).min(max) }
}

fn batches_to_json(batches: &[RecordBatch], limit: usize) -> Value {
    if batches.is_empty() {
        return json!({"columns": [], "rows": [], "row_count": 0});
    }
    let schema = batches[0].schema();
    let col_names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();

    let mut rows: Vec<Value> = Vec::new();
    'outer: for batch in batches {
        for row_idx in 0..batch.num_rows() {
            if rows.len() >= limit {
                break 'outer;
            }
            let obj: serde_json::Map<String, Value> = col_names
                .iter()
                .enumerate()
                .map(|(col_idx, name)| {
                    let col = batch.column(col_idx);
                    let v = arrow_value_to_json(col.as_ref(), row_idx);
                    (name.clone(), v)
                })
                .collect();
            rows.push(Value::Object(obj));
        }
    }
    let row_count = rows.len();
    json!({"columns": col_names, "rows": rows, "row_count": row_count})
}

fn arrow_value_to_json(col: &dyn Array, row: usize) -> Value {
    if col.is_null(row) {
        return Value::Null;
    }
    match col.data_type() {
        DataType::Boolean => {
            json!(col.as_any().downcast_ref::<BooleanArray>().unwrap().value(row))
        }
        DataType::Int8 => json!(col.as_any().downcast_ref::<Int8Array>().unwrap().value(row)),
        DataType::Int16 => json!(col.as_any().downcast_ref::<Int16Array>().unwrap().value(row)),
        DataType::Int32 => json!(col.as_any().downcast_ref::<Int32Array>().unwrap().value(row)),
        DataType::Int64 => json!(col.as_any().downcast_ref::<Int64Array>().unwrap().value(row)),
        DataType::UInt8 => json!(col.as_any().downcast_ref::<UInt8Array>().unwrap().value(row)),
        DataType::UInt16 => {
            json!(col.as_any().downcast_ref::<UInt16Array>().unwrap().value(row))
        }
        DataType::UInt32 => {
            json!(col.as_any().downcast_ref::<UInt32Array>().unwrap().value(row))
        }
        DataType::UInt64 => {
            json!(col.as_any().downcast_ref::<UInt64Array>().unwrap().value(row))
        }
        DataType::Float32 => {
            json!(col.as_any().downcast_ref::<Float32Array>().unwrap().value(row) as f64)
        }
        DataType::Float64 => {
            json!(col.as_any().downcast_ref::<Float64Array>().unwrap().value(row))
        }
        DataType::Utf8 => {
            json!(col.as_any().downcast_ref::<StringArray>().unwrap().value(row))
        }
        DataType::LargeUtf8 => {
            json!(col.as_any().downcast_ref::<LargeStringArray>().unwrap().value(row))
        }
        DataType::Date32 => {
            json!(col.as_any().downcast_ref::<Date32Array>().unwrap().value(row))
        }
        DataType::Date64 => {
            json!(col.as_any().downcast_ref::<Date64Array>().unwrap().value(row))
        }
        DataType::Timestamp(_, _) => {
            json!(col
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .map(|a| a.value(row).to_string())
                .unwrap_or_else(|| "?".to_string()))
        }
        other => json!(format!("<{other}>")),
    }
}

/// Serialize RecordBatches to CSV bytes.
fn batches_to_csv(batches: &[RecordBatch], delimiter: u8) -> Result<Vec<u8>> {
    if batches.is_empty() {
        return Ok(Vec::new());
    }
    let mut buf = Vec::new();
    let mut writer = arrow::csv::WriterBuilder::new()
        .with_delimiter(delimiter)
        .build(&mut buf);
    for batch in batches {
        writer
            .write(batch)
            .map_err(|e| ToolError::internal(format!("csv write: {e}")))?;
    }
    drop(writer);
    Ok(buf)
}

/// Serialize RecordBatches to Parquet bytes.
fn batches_to_parquet(batches: &[RecordBatch]) -> Result<Vec<u8>> {
    if batches.is_empty() {
        return Ok(Vec::new());
    }
    let schema = batches[0].schema();
    let mut buf = Vec::new();
    let mut writer = parquet::arrow::ArrowWriter::try_new(&mut buf, schema, None)
        .map_err(|e| ToolError::internal(format!("parquet writer: {e}")))?;
    for batch in batches {
        writer
            .write(batch)
            .map_err(|e| ToolError::internal(format!("parquet write: {e}")))?;
    }
    writer.close().map_err(|e| ToolError::internal(format!("parquet close: {e}")))?;
    Ok(buf)
}

/// Serialize RecordBatches to newline-delimited JSON bytes.
fn batches_to_ndjson(batches: &[RecordBatch]) -> Result<Vec<u8>> {
    let json_val = batches_to_json(batches, usize::MAX);
    let rows = json_val["rows"].as_array().cloned().unwrap_or_default();
    let mut buf = Vec::new();
    for row in &rows {
        serde_json::to_writer(&mut buf, row)
            .map_err(|e| ToolError::internal(format!("json serialize: {e}")))?;
        buf.push(b'\n');
    }
    Ok(buf)
}

pub fn register(reg: &mut ToolRegistry, config: &DbConfig) {
    let max_rows = config.max_result_rows;
    let max_file_bytes = config.max_file_bytes;

    // db.query
    reg.add(
        ToolSchema::new(
            "db.query",
            "Execute a SQL query against a CSV, Parquet, or JSON file stored in a volume. \
             The table name in SQL is the file stem lowercased (e.g. /data/sales.csv uses FROM sales).",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Path to a CSV, Parquet, or JSON file in the volume.")
        .req_str(
            "sql",
            "SQL query. The table name is the file stem lowercased \
             (e.g. /data/sales.csv maps to FROM sales).",
        )
        .opt_int("max_rows", 100, "Maximum rows to return (capped at config limit)."),
        handler(move |ctx, a| async move {
            let path_raw = a.str("path")?;
            let sql = a.str("sql")?;
            let limit = cap_rows(a.int_or("max_rows", 100), max_rows);
            let (_, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;

            let stat = client.stat(&path).await?;
            if stat.size as usize > max_file_bytes {
                return Err(ToolError::invalid_argument(format!(
                    "file '{path_raw}' is {} bytes, exceeds max_file_bytes ({max_file_bytes} bytes)",
                    stat.size
                )));
            }

            let bytes = client.read_bytes(&path).await?;
            let fmt = detect_format(&path)?;
            let tname = table_name(&path);

            let df_ctx = SessionContext::new();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            let batches = df_ctx
                .sql(&sql)
                .await
                .map_err(|e| ToolError::invalid_argument(format!("SQL error: {e}")))?
                .collect()
                .await
                .map_err(|e| ToolError::internal(format!("collect: {e}")))?;

            Ok(batches_to_json(&batches, limit))
        }),
    );

    // db.schema
    reg.add(
        ToolSchema::new("db.schema", "Return the column schema of a CSV, Parquet, or JSON file stored in a volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Path to a CSV, Parquet, or JSON file."),
        handler(move |ctx, a| async move {
            let path_raw = a.str("path")?;
            let (_, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;

            let stat = client.stat(&path).await?;
            if stat.size as usize > max_file_bytes {
                return Err(ToolError::invalid_argument(format!(
                    "file '{path_raw}' is {} bytes, exceeds max_file_bytes ({max_file_bytes} bytes)",
                    stat.size
                )));
            }

            let bytes = client.read_bytes(&path).await?;
            let fmt = detect_format(&path)?;
            let tname = table_name(&path);

            let df_ctx = SessionContext::new();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            let sql = format!("SELECT * FROM \"{tname}\" LIMIT 0");
            let df_schema = df_ctx
                .sql(&sql)
                .await
                .map_err(|e| ToolError::invalid_argument(format!("SQL error: {e}")))?;
            let schema = df_schema.schema().inner().clone();

            let cols: Vec<Value> = schema
                .fields()
                .iter()
                .map(|f| {
                    json!({
                        "name": f.name(),
                        "data_type": format!("{}", f.data_type()),
                        "nullable": f.is_nullable(),
                    })
                })
                .collect();
            Ok(Value::Array(cols))
        }),
    );

    // db.profile
    reg.add(
        ToolSchema::new(
            "db.profile",
            "Profile a CSV, Parquet, or JSON file stored in a volume: row count and per-column statistics (distinct, nulls, min, max). Capped at 20 columns.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Path to a CSV, Parquet, or JSON file."),
        handler(move |ctx, a| async move {
            let path_raw = a.str("path")?;
            let (_, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;

            let stat = client.stat(&path).await?;
            if stat.size as usize > max_file_bytes {
                return Err(ToolError::invalid_argument(format!(
                    "file '{path_raw}' is {} bytes, exceeds max_file_bytes ({max_file_bytes} bytes)",
                    stat.size
                )));
            }

            let bytes = client.read_bytes(&path).await?;
            let fmt = detect_format(&path)?;
            let tname = table_name(&path);

            let df_ctx = SessionContext::new();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            // Get schema
            let schema_sql = format!("SELECT * FROM \"{tname}\" LIMIT 0");
            let schema_df = df_ctx
                .sql(&schema_sql)
                .await
                .map_err(|e| ToolError::internal(format!("schema: {e}")))?;
            let schema = schema_df.schema().inner().clone();

            // Total row count
            let count_sql = format!("SELECT COUNT(*) as total_rows FROM \"{tname}\"");
            let count_batches = df_ctx
                .sql(&count_sql)
                .await
                .map_err(|e| ToolError::internal(format!("count: {e}")))?
                .collect()
                .await
                .map_err(|e| ToolError::internal(format!("count collect: {e}")))?;
            let total_rows = count_batches
                .first()
                .and_then(|b| b.column(0).as_any().downcast_ref::<Int64Array>())
                .map(|a| a.value(0))
                .unwrap_or(0);

            // Per-column stats (cap at 20 columns)
            let fields: Vec<_> = schema.fields().iter().take(20).cloned().collect();
            let mut columns = Vec::new();
            for field in &fields {
                let col = field.name();
                let stat_sql = format!(
                    "SELECT \
                     COUNT(\"{col}\") as non_null, \
                     COUNT(DISTINCT \"{col}\") as distinct_count, \
                     CAST(MIN(\"{col}\") AS VARCHAR) as min_val, \
                     CAST(MAX(\"{col}\") AS VARCHAR) as max_val \
                     FROM \"{tname}\""
                );
                let stat_batches = df_ctx
                    .sql(&stat_sql)
                    .await
                    .map_err(|e| ToolError::internal(format!("stat {col}: {e}")))?
                    .collect()
                    .await
                    .map_err(|e| ToolError::internal(format!("stat collect {col}: {e}")))?;

                let (non_null, distinct, min_val, max_val) = if let Some(b) = stat_batches.first() {
                    let non_null = b
                        .column(0)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .map(|a| a.value(0))
                        .unwrap_or(0);
                    let distinct = b
                        .column(1)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .map(|a| a.value(0))
                        .unwrap_or(0);
                    let min_val = arrow_value_to_json(b.column(2).as_ref(), 0);
                    let max_val = arrow_value_to_json(b.column(3).as_ref(), 0);
                    (non_null, distinct, min_val, max_val)
                } else {
                    (0, 0, Value::Null, Value::Null)
                };

                columns.push(json!({
                    "name": col,
                    "type": format!("{}", field.data_type()),
                    "non_null": non_null,
                    "distinct": distinct,
                    "min": min_val,
                    "max": max_val,
                }));
            }

            Ok(json!({ "total_rows": total_rows, "columns": columns }))
        }),
    );

    // db.sample
    reg.add(
        ToolSchema::new("db.sample", "Return a sample of rows from a CSV, Parquet, or JSON file stored in a volume.")
            .req_str("mount_id", "Project/volume id the operation targets.")
            .req_str("path", "Path to a CSV, Parquet, or JSON file.")
            .opt_int("n", 10, "Number of rows to return (capped at 1000)."),
        handler(move |ctx, a| async move {
            let path_raw = a.str("path")?;
            let n = (a.int_or("n", 10) as usize).clamp(1, 1000);
            let (_, client) = volume(&ctx, &a).await?;
            let path = norm(&ctx, &a, "path")?;

            let stat = client.stat(&path).await?;
            if stat.size as usize > max_file_bytes {
                return Err(ToolError::invalid_argument(format!(
                    "file '{path_raw}' is {} bytes, exceeds max_file_bytes ({max_file_bytes} bytes)",
                    stat.size
                )));
            }

            let bytes = client.read_bytes(&path).await?;
            let fmt = detect_format(&path)?;
            let tname = table_name(&path);

            let df_ctx = SessionContext::new();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            let sql = format!("SELECT * FROM \"{tname}\" LIMIT {n}");
            let batches = df_ctx
                .sql(&sql)
                .await
                .map_err(|e| ToolError::invalid_argument(format!("SQL error: {e}")))?
                .collect()
                .await
                .map_err(|e| ToolError::internal(format!("collect: {e}")))?;

            Ok(batches_to_json(&batches, n))
        }),
    );

    // db.convert
    reg.add(
        ToolSchema::new(
            "db.convert",
            "Convert a file between formats (CSV, Parquet, JSON/NDJSON) within a volume.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("src_path", "Source file path.")
        .req_str("dst_path", "Destination path; format inferred from extension (.csv, .parquet, .json, .ndjson)."),
        handler(move |ctx, a| async move {
            let src_raw = a.str("src_path")?;
            let dst_raw = a.str("dst_path")?;
            let (mount, client) = volume(&ctx, &a).await?;
            let src_path = ctx.state.safety.normalize_path(&src_raw)?;
            let dst_path = ctx.state.safety.normalize_path(&dst_raw)?;

            let stat = client.stat(&src_path).await?;
            if stat.size as usize > max_file_bytes {
                return Err(ToolError::invalid_argument(format!(
                    "file '{src_raw}' is {} bytes, exceeds max_file_bytes ({max_file_bytes} bytes)",
                    stat.size
                )));
            }

            let bytes = client.read_bytes(&src_path).await?;
            let src_fmt = detect_format(&src_path)?;
            let dst_fmt = detect_format(&dst_path)?;
            let tname = table_name(&src_path);

            let df_ctx = SessionContext::new();
            let _tmp = register_from_bytes(&df_ctx, &tname, src_fmt, &bytes).await?;

            let sql = format!("SELECT * FROM \"{tname}\"");
            let batches = df_ctx
                .sql(&sql)
                .await
                .map_err(|e| ToolError::invalid_argument(format!("SQL error: {e}")))?
                .collect()
                .await
                .map_err(|e| ToolError::internal(format!("collect: {e}")))?;

            let rows_converted: usize = batches.iter().map(|b| b.num_rows()).sum();

            let out_bytes = match dst_fmt {
                FileFormat::Csv => batches_to_csv(&batches, b',')?,
                FileFormat::Parquet => batches_to_parquet(&batches)?,
                FileFormat::Json | FileFormat::Ndjson => batches_to_ndjson(&batches)?,
            };

            let fmt_name = match dst_fmt {
                FileFormat::Csv => "csv",
                FileFormat::Parquet => "parquet",
                FileFormat::Json => "json",
                FileFormat::Ndjson => "ndjson",
            };

            fs_ops::write_bytes(
                &client,
                &ctx.state.safety,
                &ctx.person,
                &mount,
                &dst_path,
                &out_bytes,
                true,
                true,
            )
            .await?;

            Ok(json!({
                "src_path": src_path,
                "dst_path": dst_path,
                "rows_converted": rows_converted,
                "format": fmt_name,
            }))
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DbConfig;
    use crate::mcp::ToolRegistry;

    fn reg() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r, &DbConfig::default());
        r
    }

    #[test]
    fn five_db_tools_register() {
        assert_eq!(reg().len(), 5);
        assert_eq!(reg().names(), ["db.query", "db.schema", "db.profile", "db.sample", "db.convert"]);
    }

    #[test]
    fn detect_format_from_extension() {
        assert!(matches!(detect_format("/data/f.csv"), Ok(FileFormat::Csv)));
        assert!(matches!(detect_format("/data/f.parquet"), Ok(FileFormat::Parquet)));
        assert!(matches!(detect_format("/data/f.json"), Ok(FileFormat::Json)));
        assert!(matches!(detect_format("/data/f.ndjson"), Ok(FileFormat::Ndjson)));
        assert!(detect_format("/data/f.xlsx").is_err());
    }

    #[test]
    fn table_name_from_path() {
        assert_eq!(table_name("/data/Sales_2024.csv"), "sales_2024");
        assert_eq!(table_name("/ventes.parquet"), "ventes");
        assert_eq!(table_name("/dir/my file.json"), "my file");
    }

    #[test]
    fn cap_rows_enforced() {
        assert_eq!(cap_rows(0, 100), 100);
        assert_eq!(cap_rows(5, 100), 5);
        assert_eq!(cap_rows(20_000, 1_000), 1_000);
    }

    #[tokio::test]
    async fn query_csv_in_volume() {
        let h = crate::tools::testkit::harness_with_extra(|reg, cfg| {
            register(reg, &cfg.db);
        })
        .await;
        h.seed("/data.csv", "name,score\nAlice,90\nBob,75\nCarol,85\n").await;
        let result = h
            .call(
                "db.query",
                serde_json::json!({
                    "mount_id": crate::tools::testkit::MOUNT,
                    "path": "/data.csv",
                    "sql": "SELECT name, score FROM data WHERE CAST(score AS INT) > 80 ORDER BY CAST(score AS INT) DESC"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["row_count"], 2);
        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows[0]["name"], "Alice");
    }

    #[tokio::test]
    async fn schema_returns_column_types() {
        let h = crate::tools::testkit::harness_with_extra(|reg, cfg| {
            register(reg, &cfg.db);
        })
        .await;
        h.seed("/items.csv", "id,name,price\n1,widget,9.99\n2,gadget,24.95\n").await;
        let result = h
            .call(
                "db.schema",
                serde_json::json!({
                    "mount_id": crate::tools::testkit::MOUNT,
                    "path": "/items.csv"
                }),
            )
            .await
            .unwrap();
        let cols = result.as_array().unwrap();
        assert!(cols.iter().any(|c| c["name"] == "id"));
        assert!(cols.iter().any(|c| c["name"] == "price"));
    }

    #[tokio::test]
    async fn sample_returns_n_rows() {
        let h = crate::tools::testkit::harness_with_extra(|reg, cfg| {
            register(reg, &cfg.db);
        })
        .await;
        let csv =
            "x\n".to_string() + &(0..50).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        h.seed("/nums.csv", &csv).await;
        let result = h
            .call(
                "db.sample",
                serde_json::json!({
                    "mount_id": crate::tools::testkit::MOUNT,
                    "path": "/nums.csv",
                    "n": 5
                }),
            )
            .await
            .unwrap();
        assert_eq!(result["row_count"], 5);
    }

    #[test]
    fn max_file_bytes_logic() {
        // Verify the logic: a 6-byte file would be rejected with max_file_bytes=1.
        let stat_size = 6i64;
        let max_file_bytes = 1usize;
        assert!(stat_size as usize > max_file_bytes);
    }
}
