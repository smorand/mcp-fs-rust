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
use crate::tools::registry_support::handler;
use crate::tools::registry_support::{ToolRegistry, ToolSchema};
use crate::tools::{norm, volume};
use arrow::array::{
    Array, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, StringViewArray, UInt8Array, UInt16Array, UInt32Array,
    UInt64Array,
};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions};
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
            ctx.register_json(tname, &path_str, JsonReadOptions::default())
                .await
                .map_err(|e| ToolError::internal(format!("register json: {e}")))?;
        }
    }
    Ok(tmp)
}

/// Session with the SQL string mapping of datafusion 43: `CAST(.. AS VARCHAR)`
/// yields `Utf8`, so `db.profile` min and max stay rendered text (SPEC-0055 DR-009).
/// Since datafusion 49 the default maps VARCHAR to `Utf8View`; `arrow_value_to_json`
/// renders such a column as text either way.
fn session() -> SessionContext {
    let mut config = SessionConfig::new();
    config.options_mut().sql_parser.map_string_types_to_utf8view = false;
    SessionContext::new_with_config(config)
}

/// The `SQL error` text of datafusion 43. Newer sqlparser releases append the
/// token position (` at Line: 1, Column: 1`) to parser errors; it is dropped so
/// the message a caller sees is unchanged (SPEC-0055 DR-009).
fn sql_error(e: datafusion::error::DataFusionError) -> ToolError {
    static POSITION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        // A literal pattern known to compile: the expect cannot fire.
        regex::Regex::new(r" at Line: \d+, Column: \d+").expect("valid position pattern")
    });
    let text = e.to_string();
    ToolError::invalid_argument(format!("SQL error: {}", POSITION.replace_all(&text, "")))
}

/// Column type name shown to callers. A `Utf8View` string (what the Parquet reader
/// yields) is reported as `Utf8`, the name CSV and NDJSON give the same data
/// (SPEC-0009 E2E-019, SPEC-0057/BR-003).
fn type_name(t: &DataType) -> String {
    match t {
        DataType::Utf8View => "Utf8".to_string(),
        other => other.to_string(),
    }
}

fn cap_rows(requested: i64, config_max: usize) -> usize {
    const HARD_CAP: usize = 10_000;
    let max = config_max.min(HARD_CAP);
    if requested <= 0 { max } else { (requested as usize).min(max) }
}

/// A formatter for the column types whose JSON text is the cell text of the CSV
/// writer (SPEC-0058). The same `FormatOptions::default()` makes the two paths agree
/// by construction. `None` leaves the column to `arrow_value_to_json`.
fn text_formatter(col: &dyn Array) -> Option<ArrayFormatter<'_>> {
    match col.data_type() {
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => {
            ArrayFormatter::try_new(col, &FormatOptions::default()).ok()
        }
        _ => None,
    }
}

fn batches_to_json(batches: &[RecordBatch], limit: usize) -> Value {
    if batches.is_empty() {
        return json!({"columns": [], "rows": [], "row_count": 0});
    }
    let schema = batches[0].schema();
    let col_names: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();

    let mut rows: Vec<Value> = Vec::new();
    'outer: for batch in batches {
        // One formatter per column and batch: a timestamp formatter parses its time
        // zone when built, which would repeat for every cell otherwise.
        let formatters: Vec<Option<ArrayFormatter<'_>>> =
            batch.columns().iter().map(|c| text_formatter(c.as_ref())).collect();
        for row_idx in 0..batch.num_rows() {
            if rows.len() >= limit {
                break 'outer;
            }
            let obj: serde_json::Map<String, Value> = col_names
                .iter()
                .enumerate()
                .map(|(col_idx, name)| {
                    let col = batch.column(col_idx);
                    let v = match &formatters[col_idx] {
                        Some(f) if !col.is_null(row_idx) => json!(f.value(row_idx).to_string()),
                        _ => arrow_value_to_json(col.as_ref(), row_idx),
                    };
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
        // The Parquet reader types strings as views (SPEC-0057); same text as `Utf8`.
        DataType::Utf8View => {
            json!(col.as_any().downcast_ref::<StringViewArray>().unwrap().value(row))
        }
        DataType::LargeUtf8 => {
            json!(col.as_any().downcast_ref::<LargeStringArray>().unwrap().value(row))
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
    let mut writer = arrow::csv::WriterBuilder::new().with_delimiter(delimiter).build(&mut buf);
    for batch in batches {
        writer.write(batch).map_err(|e| ToolError::internal(format!("csv write: {e}")))?;
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
        writer.write(batch).map_err(|e| ToolError::internal(format!("parquet write: {e}")))?;
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

/// Catalog-only, `ToolRegistry`-free view of the `db.*` family.
pub fn catalog(config: &DbConfig) -> Vec<crate::tools::catalog::ToolCatalogEntry> {
    let mut reg = ToolRegistry::new();
    register(&mut reg, config);
    reg.names()
        .iter()
        .map(|n| crate::tools::catalog::ToolCatalogEntry::from(&reg.resolve(n).unwrap().schema))
        .collect()
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
        .opt_int("max_rows", 100, "Maximum rows to return (capped at config limit).")
        .read_only(true)
        .idempotent(true)
        .open_world(true),
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

            let df_ctx = session();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            let batches = df_ctx
                .sql(&sql)
                .await
                .map_err(sql_error)?
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
            .req_str("path", "Path to a CSV, Parquet, or JSON file.")
            .read_only(true)
            .idempotent(true)
            .open_world(true),
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

            let df_ctx = session();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            let sql = format!("SELECT * FROM \"{tname}\" LIMIT 0");
            let df_schema = df_ctx
                .sql(&sql)
                .await
                .map_err(sql_error)?;
            let schema = df_schema.schema().inner().clone();

            let cols: Vec<Value> = schema
                .fields()
                .iter()
                .map(|f| {
                    json!({
                        "name": f.name(),
                        "data_type": type_name(f.data_type()),
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
        .req_str("path", "Path to a CSV, Parquet, or JSON file.")
        .read_only(true)
        .idempotent(true)
        .open_world(true),
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

            let df_ctx = session();
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
                    "type": type_name(field.data_type()),
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
            .opt_int("n", 10, "Number of rows to return (capped at 1000).")
            .read_only(true)
            .idempotent(true)
            .open_world(true),
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

            let df_ctx = session();
            let _tmp = register_from_bytes(&df_ctx, &tname, fmt, &bytes).await?;

            let sql = format!("SELECT * FROM \"{tname}\" LIMIT {n}");
            let batches = df_ctx
                .sql(&sql)
                .await
                .map_err(sql_error)?
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
        .req_str(
            "dst_path",
            "Destination path; format inferred from extension (.csv, .parquet, .json, .ndjson).",
        )
        .read_only(false)
        .destructive(true)
        .idempotent(true)
        .open_world(true),
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

            let df_ctx = session();
            let _tmp = register_from_bytes(&df_ctx, &tname, src_fmt, &bytes).await?;

            let sql = format!("SELECT * FROM \"{tname}\"");
            let batches = df_ctx
                .sql(&sql)
                .await
                .map_err(sql_error)?
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

/// Fixture T1 of SPEC-0058: one row holding every date, timestamp, decimal and binary
/// kind the data tools render as text, plus a whole number and a string.
#[cfg(test)]
pub(crate) fn t1_parquet() -> Vec<u8> {
    use arrow::array::{
        BinaryArray, Date32Array, Date64Array, Decimal128Array, TimestampMicrosecondArray,
        TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
    };
    use arrow::datatypes::{Field, Schema, TimeUnit};
    use std::sync::Arc;

    let schema = Arc::new(Schema::new(vec![
        Field::new("ts_s", DataType::Timestamp(TimeUnit::Second, None), true),
        Field::new("ts_ms", DataType::Timestamp(TimeUnit::Millisecond, None), true),
        Field::new("ts_us", DataType::Timestamp(TimeUnit::Microsecond, None), true),
        Field::new("ts_ns", DataType::Timestamp(TimeUnit::Nanosecond, None), true),
        Field::new("ts_utc", DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())), true),
        Field::new("d32", DataType::Date32, true),
        Field::new("d64", DataType::Date64, true),
        Field::new("dec", DataType::Decimal128(10, 2), true),
        Field::new("bin", DataType::Binary, true),
        Field::new("n", DataType::Int64, true),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(TimestampSecondArray::from(vec![1_700_000_000])),
            Arc::new(TimestampMillisecondArray::from(vec![1_700_000_000_123])),
            Arc::new(TimestampMicrosecondArray::from(vec![1_700_000_000_123_456])),
            Arc::new(TimestampNanosecondArray::from(vec![1_700_000_000_123_456_789])),
            Arc::new(
                TimestampMicrosecondArray::from(vec![1_700_000_000_123_456]).with_timezone("UTC"),
            ),
            Arc::new(Date32Array::from(vec![19_675])),
            Arc::new(Date64Array::from(vec![1_700_000_000_000])),
            Arc::new(Decimal128Array::from(vec![12_345]).with_precision_and_scale(10, 2).unwrap()),
            Arc::new(BinaryArray::from(vec![&b"\x00\x01ab"[..]])),
            Arc::new(Int64Array::from(vec![5])),
            Arc::new(StringArray::from(vec!["Alice"])),
        ],
    )
    .unwrap();
    let mut out = Vec::new();
    let mut w = parquet::arrow::ArrowWriter::try_new(&mut out, schema, None).unwrap();
    w.write(&batch).unwrap();
    w.close().unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DbConfig;
    use crate::tools::registry_support::ToolRegistry;
    use arrow::array::Date32Array;

    fn reg() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        register(&mut r, &DbConfig::default());
        r
    }

    #[test]
    fn five_db_tools_register() {
        assert_eq!(reg().len(), 5);
        assert_eq!(
            reg().names(),
            ["db.query", "db.schema", "db.profile", "db.sample", "db.convert"]
        );
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

    const CSV: &str = "id,name,score\n1,Alice,1.5\n2,Bob,2.5\n3,Carol,4.0\n";
    const NDJSON: &str = "{\"id\":1,\"name\":\"Alice\",\"score\":1.5}\n{\"id\":2,\"name\":\"Bob\",\"score\":2.5}\n{\"id\":3,\"name\":\"Carol\",\"score\":4.0}\n";

    async fn db_harness() -> crate::tools::testkit::Harness {
        crate::tools::testkit::harness_with_extra(|reg, cfg| {
            register(reg, &cfg.db);
        })
        .await
    }

    /// Harness holding `/t.csv`, `/t.parquet` (via db.convert) and `/t.ndjson`
    /// (seeded), all with the same three rows.
    async fn three_row_harness() -> crate::tools::testkit::Harness {
        let h = db_harness().await;
        h.seed("/t.csv", CSV).await;
        h.seed("/t.ndjson", NDJSON).await;
        h.call(
            "db.convert",
            json!({"mount_id": crate::tools::testkit::MOUNT, "src_path": "/t.csv", "dst_path": "/t.parquet"}),
        )
        .await
        .unwrap();
        h
    }

    async fn call_on(
        h: &crate::tools::testkit::Harness,
        tool: &str,
        path: &str,
        extra: Value,
    ) -> Result<Value> {
        let mut args = json!({"mount_id": crate::tools::testkit::MOUNT, "path": path});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        h.call(tool, args).await
    }

    fn three_rows() -> Value {
        json!({
            "columns": ["id", "name", "score"],
            "rows": [
                {"id": 1, "name": "Alice", "score": 1.5},
                {"id": 2, "name": "Bob", "score": 2.5},
                {"id": 3, "name": "Carol", "score": 4.0}
            ],
            "row_count": 3
        })
    }

    const SELECT_ALL: &str = "SELECT id, name, score FROM t ORDER BY id";

    /// SPEC-0055/DT-001
    #[tokio::test]
    async fn spec_0055_dt_001_query_aggregate_on_csv() {
        let h = three_row_harness().await;
        let r = call_on(
            &h,
            "db.query",
            "/t.csv",
            json!({"sql": "SELECT count(*) AS n, sum(score) AS total FROM t"}),
        )
        .await
        .unwrap();
        assert_eq!(
            r,
            json!({"columns": ["n", "total"], "rows": [{"n": 3, "total": 8.0}], "row_count": 1})
        );
    }

    /// SPEC-0055/DT-002, amended by SPEC-0057/BR-001: a Parquet written by
    /// db.convert reads its string column back as `Utf8View`, which the row
    /// serializer renders as the stored text.
    #[tokio::test]
    async fn spec_0055_dt_002_query_parquet() {
        let h = three_row_harness().await;
        let r = call_on(&h, "db.query", "/t.parquet", json!({"sql": SELECT_ALL})).await.unwrap();
        assert_eq!(
            r,
            json!({
                "columns": ["id", "name", "score"],
                "rows": [
                    {"id": 1, "name": "Alice", "score": 1.5},
                    {"id": 2, "name": "Bob", "score": 2.5},
                    {"id": 3, "name": "Carol", "score": 4.0}
                ],
                "row_count": 3
            })
        );
    }

    /// SPEC-0055/DT-003
    #[tokio::test]
    async fn spec_0055_dt_003_query_ndjson() {
        let h = three_row_harness().await;
        let r = call_on(&h, "db.query", "/t.ndjson", json!({"sql": SELECT_ALL})).await.unwrap();
        assert_eq!(r, three_rows());
    }

    fn schema_of(types: [&str; 3]) -> Value {
        json!([
            {"name": "id", "data_type": types[0], "nullable": true},
            {"name": "name", "data_type": types[1], "nullable": true},
            {"name": "score", "data_type": types[2], "nullable": true}
        ])
    }

    /// SPEC-0055/DT-004, amended by SPEC-0057/BR-003: the Parquet string column
    /// reports `Utf8`, like the CSV and the NDJSON.
    #[tokio::test]
    async fn spec_0055_dt_004_schema_type_names() {
        let h = three_row_harness().await;
        let csv = call_on(&h, "db.schema", "/t.csv", json!({})).await.unwrap();
        assert_eq!(csv, schema_of(["Int64", "Utf8", "Float64"]));
        let parquet = call_on(&h, "db.schema", "/t.parquet", json!({})).await.unwrap();
        assert_eq!(parquet, schema_of(["Int64", "Utf8", "Float64"]));
        let ndjson = call_on(&h, "db.schema", "/t.ndjson", json!({})).await.unwrap();
        assert_eq!(ndjson, schema_of(["Int64", "Utf8", "Float64"]));
    }

    fn profile_of(name_type: &str) -> Value {
        json!({
            "total_rows": 3,
            "columns": [
                {"name": "id", "type": "Int64", "non_null": 3, "distinct": 3, "min": "1", "max": "3"},
                {"name": "name", "type": name_type, "non_null": 3, "distinct": 3, "min": "Alice", "max": "Carol"},
                {"name": "score", "type": "Float64", "non_null": 3, "distinct": 3, "min": "1.5", "max": "4.0"}
            ]
        })
    }

    /// SPEC-0055/DT-005
    #[tokio::test]
    async fn spec_0055_dt_005_profile_csv() {
        let h = three_row_harness().await;
        let r = call_on(&h, "db.profile", "/t.csv", json!({})).await.unwrap();
        assert_eq!(r, profile_of("Utf8"));
    }

    /// SPEC-0055/DT-006, amended by SPEC-0057/BR-004: same profile as the CSV.
    #[tokio::test]
    async fn spec_0055_dt_006_profile_parquet() {
        let h = three_row_harness().await;
        let r = call_on(&h, "db.profile", "/t.parquet", json!({})).await.unwrap();
        assert_eq!(r, profile_of("Utf8"));
    }

    /// SPEC-0055/DT-007: the first two rows, in file order, on 43.
    #[tokio::test]
    async fn spec_0055_dt_007_sample_two_rows_of_ndjson() {
        let h = three_row_harness().await;
        let r = call_on(&h, "db.sample", "/t.ndjson", json!({"n": 2})).await.unwrap();
        assert_eq!(
            r,
            json!({
                "columns": ["id", "name", "score"],
                "rows": [
                    {"id": 1, "name": "Alice", "score": 1.5},
                    {"id": 2, "name": "Bob", "score": 2.5}
                ],
                "row_count": 2
            })
        );
    }

    async fn convert(h: &crate::tools::testkit::Harness, src: &str, dst: &str) -> Value {
        h.call(
            "db.convert",
            json!({"mount_id": crate::tools::testkit::MOUNT, "src_path": src, "dst_path": dst}),
        )
        .await
        .unwrap()
    }

    /// SPEC-0055/DT-008
    #[tokio::test]
    async fn spec_0055_dt_008_convert_csv_to_parquet_to_csv() {
        let h = three_row_harness().await;
        let r = convert(&h, "/t.parquet", "/back.csv").await;
        assert_eq!(
            r,
            json!({"src_path": "/t.parquet", "dst_path": "/back.csv", "rows_converted": 3, "format": "csv"})
        );
        let text = h.client().await.read_text("/back.csv").await.unwrap();
        assert_eq!(text, "id,name,score\n1,Alice,1.5\n2,Bob,2.5\n3,Carol,4.0\n");
    }

    /// SPEC-0055/DT-009
    #[tokio::test]
    async fn spec_0055_dt_009_convert_csv_to_ndjson() {
        let h = three_row_harness().await;
        let r = convert(&h, "/t.csv", "/out.ndjson").await;
        assert_eq!(
            r,
            json!({"src_path": "/t.csv", "dst_path": "/out.ndjson", "rows_converted": 3, "format": "ndjson"})
        );
        let text = h.client().await.read_text("/out.ndjson").await.unwrap();
        assert_eq!(
            text,
            "{\"id\":1,\"name\":\"Alice\",\"score\":1.5}\n{\"id\":2,\"name\":\"Bob\",\"score\":2.5}\n{\"id\":3,\"name\":\"Carol\",\"score\":4.0}\n"
        );
    }

    /// SPEC-0055/DT-010. The recorded text carries no path or version noise, so
    /// it is asserted whole.
    #[tokio::test]
    async fn spec_0055_dt_010_query_syntax_error_text() {
        let h = three_row_harness().await;
        let e = call_on(&h, "db.query", "/t.csv", json!({"sql": "SELEKT 1"})).await.unwrap_err();
        assert_eq!(e.code, "ERR_INVALID_ARGUMENT");
        assert_eq!(
            e.message,
            "SQL error: SQL error: ParserError(\"Expected: an SQL statement, found: SELEKT\")"
        );
    }

    /// SPEC-0055/DT-011
    #[tokio::test]
    async fn spec_0055_dt_011_query_max_rows_caps_the_payload() {
        let h = three_row_harness().await;
        let r = call_on(
            &h,
            "db.query",
            "/t.csv",
            json!({"sql": "SELECT id FROM t ORDER BY id", "max_rows": 2}),
        )
        .await
        .unwrap();
        assert_eq!(r["rows"].as_array().unwrap().len(), 2);
        assert_eq!(r["row_count"], 2);
        let mut keys: Vec<&String> = r.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["columns", "row_count", "rows"]);
        assert_eq!(r, json!({"columns": ["id"], "rows": [{"id": 1}, {"id": 2}], "row_count": 2}));
    }

    /// A Parquet file with zstd compressed pages, rows (1, Alice) and (2, Bob),
    /// written without a dictionary so each column chunk starts with its data
    /// page. Returns the bytes and the (start, end) of column 0's chunk.
    fn zstd_parquet() -> (Vec<u8>, (usize, usize)) {
        use arrow::array::{Int64Array, StringArray};
        use arrow::datatypes::{Field, Schema};
        use parquet::arrow::ArrowWriter;
        use parquet::basic::{Compression, ZstdLevel};
        use parquet::file::properties::WriterProperties;
        use parquet::file::reader::{FileReader, SerializedFileReader};

        let schema = std::sync::Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                std::sync::Arc::new(Int64Array::from(vec![1, 2])),
                std::sync::Arc::new(StringArray::from(vec!["Alice", "Bob"])),
            ],
        )
        .unwrap();
        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            .set_dictionary_enabled(false)
            .build();
        let mut out = Vec::new();
        let mut w = ArrowWriter::try_new(&mut out, schema, Some(props)).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();

        let reader = SerializedFileReader::new(bytes::Bytes::from(out.clone())).unwrap();
        let col = reader.metadata().row_group(0).column(0);
        assert!(matches!(col.compression(), Compression::ZSTD(_)));
        let (start, len) = col.byte_range();
        (out, (start as usize, (start + len) as usize))
    }

    fn id_name_rows() -> Value {
        json!({
            "columns": ["id", "name"],
            "rows": [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}],
            "row_count": 2
        })
    }

    /// SPEC-0055/DT-040. Effect not observable by assertion (memory class); this
    /// proves the zstd read path works. The string column renders as text.
    #[tokio::test]
    async fn spec_0055_dt_040_zstd_parquet_is_read() {
        let h = db_harness().await;
        let (bytes, _) = zstd_parquet();
        h.client().await.write_bytes_atomic("/t.parquet", &bytes).await.unwrap();
        let sql = json!({"sql": "SELECT id, name FROM t ORDER BY id"});
        let first = call_on(&h, "db.query", "/t.parquet", sql.clone()).await.unwrap();
        assert_eq!(first, id_name_rows());
        let second = call_on(&h, "db.query", "/t.parquet", sql).await.unwrap();
        assert_eq!(second, first);
    }

    /// SPEC-0055/DT-041. The first data page of `id` becomes the literal bytes
    /// 22 B5 2F FD 00 00 then zeros, page header and file length untouched.
    /// A rejection guard: the error comes from the page decode, not from the
    /// footer, which is intact.
    #[tokio::test]
    async fn spec_0055_dt_041_corrupt_zstd_page_is_rejected() {
        let h = db_harness().await;
        let (valid, (start, end)) = zstd_parquet();
        let client = h.client().await;
        client.write_bytes_atomic("/t.parquet", &valid).await.unwrap();

        // The page body begins at the zstd frame magic that follows the page header.
        let body = start
            + valid[start..end].windows(4).position(|w| w == [0x28, 0xB5, 0x2F, 0xFD]).unwrap();
        let mut corrupt = valid.clone();
        corrupt[body..end].fill(0);
        corrupt[body..body + 6].copy_from_slice(&[0x22, 0xB5, 0x2F, 0xFD, 0x00, 0x00]);
        assert_eq!(corrupt.len(), valid.len());
        client.write_bytes_atomic("/bad.parquet", &corrupt).await.unwrap();

        let err = call_on(&h, "db.query", "/bad.parquet", json!({"sql": "SELECT * FROM bad"}))
            .await
            .unwrap_err();
        assert!(!err.message.is_empty());

        let ok = call_on(
            &h,
            "db.query",
            "/t.parquet",
            json!({"sql": "SELECT id, name FROM t ORDER BY id"}),
        )
        .await
        .unwrap();
        assert_eq!(ok, id_name_rows());
    }

    /// SPEC-0057/BT-010: the three formats report the same column types.
    #[tokio::test]
    async fn bt_0057_010_schema_type_of_a_parquet_string_column() {
        let h = three_row_harness().await;
        for path in ["/t.csv", "/t.parquet", "/t.ndjson"] {
            let r = call_on(&h, "db.schema", path, json!({})).await.unwrap();
            assert_eq!(r, schema_of(["Int64", "Utf8", "Float64"]), "{path}");
        }
    }

    /// SPEC-0057/BT-011: the Parquet profile equals the CSV profile.
    #[tokio::test]
    async fn bt_0057_011_profile_of_a_parquet_string_column() {
        let h = three_row_harness().await;
        let r = call_on(&h, "db.profile", "/t.parquet", json!({})).await.unwrap();
        assert_eq!(r, profile_of("Utf8"));
    }

    /// A Parquet with one nullable string column `s`.
    fn string_parquet(values: &[Option<&str>]) -> Vec<u8> {
        use arrow::datatypes::{Field, Schema};
        use parquet::arrow::ArrowWriter;

        let schema = std::sync::Arc::new(Schema::new(vec![Field::new("s", DataType::Utf8, true)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![std::sync::Arc::new(StringArray::from(values.to_vec()))],
        )
        .unwrap();
        let mut out = Vec::new();
        let mut w = ArrowWriter::try_new(&mut out, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        out
    }

    /// SPEC-0057/BT-012: null, empty, long and non ASCII strings survive.
    #[tokio::test]
    async fn bt_0057_012_edge_values_survive_a_parquet_round_trip() {
        let h = db_harness().await;
        let bytes = string_parquet(&[
            Some("h\u{e9}llo \u{65e5}\u{672c}"),
            Some(""),
            None,
            Some("a string well over twelve bytes"),
        ]);
        h.client().await.write_bytes_atomic("/t.parquet", &bytes).await.unwrap();
        let expected = json!([
            {"s": "h\u{e9}llo \u{65e5}\u{672c}"},
            {"s": ""},
            {"s": null},
            {"s": "a string well over twelve bytes"}
        ]);
        let r =
            call_on(&h, "db.query", "/t.parquet", json!({"sql": "SELECT s FROM t"})).await.unwrap();
        assert_eq!(r["rows"], expected);
        convert(&h, "/t.parquet", "/o.ndjson").await;
        let text = h.client().await.read_text("/o.ndjson").await.unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(Value::Array(lines), expected);
        assert!(text.ends_with('\n'));
    }

    /// SPEC-0057/BT-013: a string computed from a Parquet column, and a sample.
    #[tokio::test]
    async fn bt_0057_013_computed_and_sampled_strings() {
        let h = three_row_harness().await;
        let r = call_on(
            &h,
            "db.query",
            "/t.parquet",
            json!({"sql": "SELECT upper(name) AS u FROM t ORDER BY id"}),
        )
        .await
        .unwrap();
        assert_eq!(r["rows"], json!([{"u": "ALICE"}, {"u": "BOB"}, {"u": "CAROL"}]));
        let r = call_on(&h, "db.sample", "/t.parquet", json!({"n": 2})).await.unwrap();
        assert_eq!(
            r["rows"],
            json!([
                {"id": 1, "name": "Alice", "score": 1.5},
                {"id": 2, "name": "Bob", "score": 2.5}
            ])
        );
    }

    /// A one column Parquet file written with the arrow writer.
    fn one_column_parquet(array: arrow::array::ArrayRef, name: &str) -> Vec<u8> {
        use arrow::datatypes::{Field, Schema};
        let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
            name,
            array.data_type().clone(),
            true,
        )]));
        let batch = RecordBatch::try_new(schema.clone(), vec![array]).unwrap();
        let mut out = Vec::new();
        let mut w = parquet::arrow::ArrowWriter::try_new(&mut out, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        out
    }

    async fn put(h: &crate::tools::testkit::Harness, path: &str, bytes: &[u8]) {
        h.client().await.write_bytes_atomic(path, bytes).await.unwrap();
    }

    async fn t1_harness() -> crate::tools::testkit::Harness {
        let h = db_harness().await;
        put(&h, "/t.parquet", &t1_parquet()).await;
        h
    }

    /// SPEC-0058/E2E-002
    #[tokio::test]
    async fn bt_0058_e2e_002_dates_read_as_text() {
        let h = t1_harness().await;
        let r = call_on(&h, "db.query", "/t.parquet", json!({"sql": "SELECT d32, d64 FROM t"}))
            .await
            .unwrap();
        assert_eq!(r["rows"], json!([{"d32": "2023-11-14", "d64": "2023-11-14T22:13:20"}]));
    }

    /// SPEC-0058/E2E-005
    #[tokio::test]
    async fn bt_0058_e2e_005_dates_in_a_csv_file_read_as_text() {
        let h = db_harness().await;
        h.seed("/c.csv", "d,ts,x\n2023-11-14,2023-11-14T22:13:20,1.50\n,,2.50\n").await;
        let expected = json!([
            {"d": "2023-11-14", "ts": "2023-11-14T22:13:20", "x": 1.5},
            {"d": null, "ts": null, "x": 2.5}
        ]);
        let s = call_on(&h, "db.sample", "/c.csv", json!({"n": 5})).await.unwrap();
        assert_eq!(s["rows"], expected);
        let q =
            call_on(&h, "db.query", "/c.csv", json!({"sql": "SELECT d, ts, x FROM c ORDER BY x"}))
                .await
                .unwrap();
        assert_eq!(q["rows"], expected);
    }

    /// SPEC-0058/E2E-009
    #[tokio::test]
    async fn bt_0058_e2e_009_moments_before_1970() {
        use arrow::array::TimestampSecondArray;
        use arrow::datatypes::{Field, Schema, TimeUnit};
        let h = db_harness().await;
        let schema = std::sync::Arc::new(Schema::new(vec![
            Field::new("pre", DataType::Timestamp(TimeUnit::Second, None), true),
            Field::new("dpre", DataType::Date32, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                std::sync::Arc::new(TimestampSecondArray::from(vec![-1])),
                std::sync::Arc::new(Date32Array::from(vec![-1])),
            ],
        )
        .unwrap();
        let mut out = Vec::new();
        let mut w = parquet::arrow::ArrowWriter::try_new(&mut out, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        put(&h, "/p.parquet", &out).await;
        let r =
            call_on(&h, "db.query", "/p.parquet", json!({"sql": "SELECT * FROM p"})).await.unwrap();
        assert_eq!(r["rows"], json!([{"pre": "1969-12-31T23:59:59", "dpre": "1969-12-31"}]));
    }

    /// SPEC-0058/E2E-010
    #[tokio::test]
    async fn bt_0058_e2e_010_a_time_zone_with_an_offset() {
        use arrow::array::TimestampMicrosecondArray;
        let h = db_harness().await;
        let array =
            TimestampMicrosecondArray::from(vec![1_700_000_000_123_456]).with_timezone("+02:00");
        put(&h, "/z.parquet", &one_column_parquet(std::sync::Arc::new(array), "tz2")).await;
        let r = call_on(&h, "db.query", "/z.parquet", json!({"sql": "SELECT tz2 FROM z"}))
            .await
            .unwrap();
        assert_eq!(r["rows"], json!([{"tz2": "2023-11-15T00:13:20.123456+02:00"}]));
        convert(&h, "/z.parquet", "/z.csv").await;
        let csv = h.client().await.read_text("/z.csv").await.unwrap();
        assert_eq!(csv.lines().nth(1), Some("2023-11-15T00:13:20.123456+02:00"));
    }

    /// A Parquet file with a list column `lst` ([1, 2], or missing), a date column and, when asked, a whole number column `k`.
    fn list_parquet(second_row_missing: bool, with_k: bool) -> Vec<u8> {
        use arrow::array::ListArray;
        use arrow::datatypes::{Field, Int64Type, Schema};
        let rows: Vec<Option<Vec<Option<i64>>>> = if second_row_missing {
            vec![Some(vec![Some(1), Some(2)]), None]
        } else {
            vec![Some(vec![Some(1), Some(2)])]
        };
        let n = rows.len();
        let lst = ListArray::from_iter_primitive::<Int64Type, _, _>(rows);
        let d32 = Date32Array::from(vec![Some(19_675); n]);
        let mut fields = vec![
            Field::new("lst", lst.data_type().clone(), true),
            Field::new("d32", DataType::Date32, true),
        ];
        let mut columns: Vec<arrow::array::ArrayRef> =
            vec![std::sync::Arc::new(lst), std::sync::Arc::new(d32)];
        if with_k {
            fields.push(Field::new("k", DataType::Int64, true));
            columns.push(std::sync::Arc::new(Int64Array::from(vec![7; n])));
        }
        let schema = std::sync::Arc::new(Schema::new(fields));
        let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
        let mut out = Vec::new();
        let mut w = parquet::arrow::ArrowWriter::try_new(&mut out, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        out
    }

    /// SPEC-0058/E2E-012: protects, passes before the change for `lst`, fails for `d32`.
    #[tokio::test]
    async fn bt_0058_e2e_012_a_nested_column_keeps_its_placeholder() {
        let h = db_harness().await;
        put(&h, "/l.parquet", &list_parquet(false, false)).await;
        let schema = call_on(&h, "db.schema", "/l.parquet", json!({})).await.unwrap();
        assert_eq!(schema[0]["name"], "lst");
        assert_eq!(schema[0]["data_type"], "List(Int64)");
        let r = call_on(&h, "db.query", "/l.parquet", json!({"sql": "SELECT lst, d32 FROM l"}))
            .await
            .unwrap();
        assert_eq!(r["rows"], json!([{"lst": "<List(Int64)>", "d32": "2023-11-14"}]));
        let src = "/l.parquet";
        let c = h
            .call(
                "db.convert",
                json!({"mount_id": crate::tools::testkit::MOUNT, "src_path": src, "dst_path": "/l.ndjson"}),
            )
            .await
            .unwrap();
        assert_eq!(c["format"], "ndjson");
        let text = h.client().await.read_text("/l.ndjson").await.unwrap();
        assert_eq!(text, "{\"lst\":\"<List(Int64)>\",\"d32\":\"2023-11-14\"}\n");
    }

    /// SPEC-0058/E2E-013: protects.
    #[tokio::test]
    async fn bt_0058_e2e_013_plain_columns_do_not_move() {
        let h = three_row_harness().await;
        let expected = json!({
            "columns": ["id", "name", "score"],
            "rows": [
                {"id": 1, "name": "Alice", "score": 1.5},
                {"id": 2, "name": "Bob", "score": 2.5},
                {"id": 3, "name": "Carol", "score": 4.0}
            ],
            "row_count": 3
        });
        for path in ["/t.csv", "/t.ndjson", "/t.parquet"] {
            let r = call_on(&h, "db.query", path, json!({"sql": SELECT_ALL})).await.unwrap();
            assert_eq!(r, expected, "{path}");
        }
        put(&h, "/l.parquet", &list_parquet(false, true)).await;
        let r =
            call_on(&h, "db.query", "/l.parquet", json!({"sql": "SELECT k FROM l"})).await.unwrap();
        assert_eq!(r, json!({"columns": ["k"], "rows": [{"k": 7}], "row_count": 1}));
    }

    /// SPEC-0058/E2E-015: protects.
    #[tokio::test]
    async fn bt_0058_e2e_015_a_missing_nested_value_is_null() {
        let h = db_harness().await;
        put(&h, "/m.parquet", &list_parquet(true, false)).await;
        let r = call_on(&h, "db.query", "/m.parquet", json!({"sql": "SELECT lst FROM m"}))
            .await
            .unwrap();
        assert_eq!(r["rows"], json!([{"lst": "<List(Int64)>"}, {"lst": null}]));
    }

    #[test]
    fn max_file_bytes_logic() {
        // Verify the logic: a 6-byte file would be rejected with max_file_bytes=1.
        let stat_size = 6i64;
        let max_file_bytes = 1usize;
        assert!(stat_size as usize > max_file_bytes);
    }
}
