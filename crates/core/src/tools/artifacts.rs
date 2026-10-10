//! Document artifacts (SPEC-0019): the images and tables a conversion kept,
//! listed per document and read one by one.
//!
//! Typed functions only: the `#[tool]` methods in `mcp::server` authorize and
//! normalize the path, then call straight through to [`list_images`],
//! [`get_image`], [`list_tables`], [`get_table`] and [`document_view`], so
//! every door shares this one implementation (DEC-001).

use crate::docs::artifacts::render::{self, TableFormat, TableMode, ViewImage, ViewTable};
use crate::docs::artifacts::{
    ARTIFACT_PAGE_SIZE, CsvQuality, ListKind, NO_ARTIFACTS, decode_marker, encode_marker,
};
use crate::errors::{Result, ToolError};
use crate::state::AppState;
use crate::storage::VolumeClient;
use crate::storage::meta::RelationalMetaStore;
#[cfg(test)]
use crate::tools::authorize_only;
#[cfg(test)]
use crate::tools::registry_support::{ToolRegistry, ToolSchema, handler};
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::Arc;

/// Test-dispatch glue for the artifact tools, mirroring the production
/// handlers (`McpServer::fs_list_images` and siblings) so the golden contract
/// sees the same schemas.
#[cfg(test)]
pub(crate) fn register(reg: &mut ToolRegistry) {
    reg.add(
        ToolSchema::new(
            "fs.list_images",
            "List the images kept from a document's last conversion, in reading order, 100 per page.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the source document.")
        .opt_str_null(
            "marker",
            "Continuation marker returned by the previous page; omit for the first page.",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let path = crate::tools::norm(&ctx, &a, "path")?;
            list_images(&ctx.state, &mount, &path, a.opt_str("marker").as_deref()).await
        }),
    );
    reg.add(
        ToolSchema::new(
            "fs.get_image",
            "Get one image kept from a document's last conversion: its picture as base64, its format and its caption.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the source document.")
        .req_str("id", "Image id from fs.list_images, e.g. image-1.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let path = crate::tools::norm(&ctx, &a, "path")?;
            let id = a.str("id")?;
            get_image(&ctx.state, &mount, &path, &id).await
        }),
    );
    reg.add(
        ToolSchema::new(
            "fs.list_tables",
            "List the tables kept from a document's last conversion, in reading order, 100 per page.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the source document.")
        .opt_str_null(
            "marker",
            "Continuation marker returned by the previous page; omit for the first page.",
        )
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let path = crate::tools::norm(&ctx, &a, "path")?;
            list_tables(&ctx.state, &mount, &path, a.opt_str("marker").as_deref()).await
        }),
    );
    reg.add(
        ToolSchema::new(
            "fs.get_table",
            "Get one table kept from a document's last conversion, as Markdown or CSV.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the source document.")
        .req_str("id", "Table id from fs.list_tables, e.g. table-1.")
        .opt_str_null("format", "markdown (default) or csv.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let path = crate::tools::norm(&ctx, &a, "path")?;
            let id = a.str("id")?;
            get_table(&ctx.state, &mount, &path, &id, a.opt_str("format").as_deref()).await
        }),
    );
    reg.add(
        ToolSchema::new(
            "fs.get_document_view",
            "Get a converted document's whole text with an image line at each image and each table in the chosen table mode.",
        )
        .req_str("mount_id", "Project/volume id the operation targets.")
        .req_str("path", "Absolute POSIX path of the source document.")
        .opt_bool("captions", true, "Show each image's caption in its image line.")
        .opt_str_null("table_mode", "markdown (default), csv-reference or both.")
        .read_only(true)
        .idempotent(true)
        .open_world(false),
        handler(|ctx, a| async move {
            let mount = authorize_only(&ctx, &a).await?;
            let path = crate::tools::norm(&ctx, &a, "path")?;
            let captions = a.bool_or("captions", true);
            let mode = a.opt_str("table_mode");
            document_view(&ctx.state, &mount, &path, captions, mode.as_deref()).await
        }),
    );
}

/// `fs.list_images(mount_id, path, marker)` (FR-NEW-003). The caller has
/// authorized `mount_id` and normalized `path`; the remaining refusals come in
/// the FR-NEW-031 order: not a file, invalid marker, no artifacts.
///
/// Time: O(1) queries, one pointer lookup joined with the node's rev and one
/// page of at most `ARTIFACT_PAGE_SIZE + 1` rows. Space: O(page).
pub(crate) async fn list_images(
    state: &AppState,
    mount_id: &str,
    path: &str,
    marker: Option<&str>,
) -> Result<Value> {
    let (store, set_id, offset) =
        current_page(state, mount_id, path, marker, ListKind::Images).await?;
    // An offset beyond what SQL can page is past the end: an empty page.
    let Ok(sql_offset) = i64::try_from(offset) else {
        return Ok(json!({"images": [], "marker": null}));
    };
    let mut rows =
        store.list_artifact_images(&set_id, ARTIFACT_PAGE_SIZE as i64 + 1, sql_offset).await?;
    let more = rows.len() > ARTIFACT_PAGE_SIZE;
    rows.truncate(ARTIFACT_PAGE_SIZE);
    let images: Vec<Value> = rows
        .into_iter()
        .map(|i| json!({"id": format!("image-{}", i.seq), "caption": i.caption, "page": i.page}))
        .collect();
    let next =
        more.then(|| encode_marker(mount_id, ListKind::Images, path, offset + ARTIFACT_PAGE_SIZE));
    Ok(json!({"images": images, "marker": next}))
}

/// `fs.get_image(mount_id, path, id)` (FR-NEW-004): the picture byte for byte
/// as the converter extracted it, base64 encoded. Refusals in the FR-NEW-031
/// order: not a file, no artifacts, unknown id.
///
/// Time: O(1) queries plus one blob read. Space: O(picture).
pub(crate) async fn get_image(
    state: &AppState,
    mount_id: &str,
    path: &str,
    id: &str,
) -> Result<Value> {
    let client = state.stores.client(mount_id).await?;
    ensure_file(&client, path).await?;
    let (store, set_id) = current_set(&client, path).await?;
    let not_found = || ToolError::not_found(format!("Not found: {id}"));
    let seq = artifact_seq("image-", id).ok_or_else(not_found)?;
    let image = store.artifact_image(&set_id, seq).await?.ok_or_else(not_found)?;
    let bytes = client.blob.get(&image.sha256, 0, None).await?;
    Ok(json!({
        "id": id,
        "format": image.format,
        "caption": image.caption,
        "page": image.page,
        "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    }))
}

/// The meta store, current set and page offset of a list request, refused in
/// the FR-NEW-031 order: not a file, invalid marker, no artifacts.
async fn current_page(
    state: &AppState,
    mount_id: &str,
    path: &str,
    marker: Option<&str>,
    kind: ListKind,
) -> Result<(Arc<RelationalMetaStore>, String, usize)> {
    let client = state.stores.client(mount_id).await?;
    ensure_file(&client, path).await?;
    let offset = match marker {
        Some(raw) => decode_marker(raw, mount_id, kind, path)?,
        None => 0,
    };
    let (store, set_id) = current_set(&client, path).await?;
    Ok((store, set_id, offset))
}

/// `not a file: <path>` unless `path` holds a file (FR-NEW-028).
async fn ensure_file(client: &VolumeClient, path: &str) -> Result<()> {
    if client.is_file(path).await? {
        Ok(())
    } else {
        Err(ToolError::not_found(format!("not a file: {path}")))
    }
}

/// The meta store and the id of the set `path` currently points at, or the
/// no artifacts refusal (FR-NEW-023).
async fn current_set(
    client: &VolumeClient,
    path: &str,
) -> Result<(Arc<RelationalMetaStore>, String)> {
    let store = client
        .trash
        .clone()
        .ok_or_else(|| ToolError::internal("volume has no relational meta store attached"))?;
    let set_id = store
        .current_artifact_set(path)
        .await?
        .ok_or_else(|| ToolError::not_found(NO_ARTIFACTS))?;
    Ok((store, set_id))
}

/// `fs.list_tables(mount_id, path, marker)` (FR-NEW-005). The caller has
/// authorized `mount_id` and normalized `path`; the remaining refusals come in
/// the FR-NEW-031 order: not a file, invalid marker, no artifacts.
///
/// Time: O(1) queries, one pointer lookup joined with the node's rev and one
/// page of at most `ARTIFACT_PAGE_SIZE + 1` rows. Space: O(page).
pub(crate) async fn list_tables(
    state: &AppState,
    mount_id: &str,
    path: &str,
    marker: Option<&str>,
) -> Result<Value> {
    let (store, set_id, offset) =
        current_page(state, mount_id, path, marker, ListKind::Tables).await?;
    // An offset beyond what SQL can page is past the end: an empty page.
    let Ok(sql_offset) = i64::try_from(offset) else {
        return Ok(json!({"tables": [], "marker": null}));
    };
    let mut rows =
        store.list_artifact_tables(&set_id, ARTIFACT_PAGE_SIZE as i64 + 1, sql_offset).await?;
    let more = rows.len() > ARTIFACT_PAGE_SIZE;
    rows.truncate(ARTIFACT_PAGE_SIZE);
    let tables: Vec<Value> = rows
        .into_iter()
        .map(|t| {
            json!({
                "id": format!("table-{}", t.seq),
                "caption": t.caption,
                "rows": t.rows,
                "columns": t.cols,
                "csv_quality": CsvQuality::label(&t.quality),
            })
        })
        .collect();
    let next =
        more.then(|| encode_marker(mount_id, ListKind::Tables, path, offset + ARTIFACT_PAGE_SIZE));
    Ok(json!({"tables": tables, "marker": next}))
}

/// `fs.get_table(mount_id, path, id, format)` (FR-NEW-006). The caller has
/// authorized `mount_id` and normalized `path`; the remaining refusals come in
/// the FR-NEW-031 order: not a file, unsupported format, no artifacts, unknown
/// id.
///
/// Time: O(1) queries plus O(cells) to decode and render the one table.
/// Space: O(cells).
pub(crate) async fn get_table(
    state: &AppState,
    mount_id: &str,
    path: &str,
    id: &str,
    format: Option<&str>,
) -> Result<Value> {
    let client = state.stores.client(mount_id).await?;
    ensure_file(&client, path).await?;
    let format = TableFormat::parse(format)?;
    let (store, set_id) = current_set(&client, path).await?;
    let not_found = || ToolError::not_found(format!("Not found: {id}"));
    let seq = artifact_seq("table-", id).ok_or_else(not_found)?;
    let raw = store.artifact_table_cells(&set_id, seq).await?.ok_or_else(not_found)?;
    let cells: Vec<Vec<String>> = serde_json::from_str(&raw)
        .map_err(|e| ToolError::internal(format!("stored table cells: {e}")))?;
    Ok(json!({"id": id, "format": format.as_str(), "content": format.render(&cells)}))
}

/// `fs.get_document_view(mount_id, path, captions, table_mode)` (FR-NEW-007,
/// FR-NEW-008): the conversion text of the current set with its image lines
/// and its tables in `table_mode`. Built from the stored conversion text only,
/// so the Markdown sibling is never read nor written (FR-NEW-009, FR-NEW-022).
/// Refusals in the FR-NEW-031 order: not a file, unsupported mode, no
/// artifacts.
///
/// Time: O(1) queries, one blob read, then O(text + cells) to assemble.
/// Space: O(text + cells).
pub(crate) async fn document_view(
    state: &AppState,
    mount_id: &str,
    path: &str,
    captions: bool,
    table_mode: Option<&str>,
) -> Result<Value> {
    let client = state.stores.client(mount_id).await?;
    ensure_file(&client, path).await?;
    let mode = TableMode::parse(table_mode)?;
    let (store, set_id) = current_set(&client, path).await?;
    let text = match store.artifact_text_sha(&set_id).await? {
        Some(sha) => String::from_utf8(client.blob.get(&sha, 0, None).await?)
            .map_err(|e| ToolError::internal(format!("stored conversion text: {e}")))?,
        None => String::new(),
    };
    let images: Vec<ViewImage> = store
        .artifact_view_images(&set_id)
        .await?
        .into_iter()
        .map(|i| ViewImage { seq: i.seq, caption: i.caption, line: position(i.line) })
        .collect();
    let tables = store
        .artifact_view_tables(&set_id)
        .await?
        .into_iter()
        .map(|t| {
            let cells: Vec<Vec<String>> = serde_json::from_str(&t.cells)
                .map_err(|e| ToolError::internal(format!("stored table cells: {e}")))?;
            Ok(ViewTable {
                seq: t.seq,
                caption: t.caption,
                cells,
                line_start: position(t.line_start),
                line_count: position(t.line_count),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let text = render::document_view(&text, &images, &tables, captions, mode);
    Ok(json!({"path": path, "text": text}))
}

/// A stored line position; a negative one (never written) lands past any
/// text, so the view ignores it.
fn position(stored: i64) -> usize {
    usize::try_from(stored).unwrap_or(usize::MAX)
}

/// The `n` of a canonical `<prefix><n>` id (`table-<n>`, `image-<n>`), `n`
/// at least 1 with no sign or leading zero, so `table-01` is not an alias of
/// `table-1`.
fn artifact_seq(prefix: &str, id: &str) -> Option<i64> {
    let digits = id.strip_prefix(prefix)?;
    let n: i64 = digits.parse().ok()?;
    (n >= 1 && n.to_string() == digits).then_some(n)
}

#[cfg(test)]
mod e2e {
    //! SPEC-0019 acceptance tests, driven through the real `McpServer` tool
    //! methods, the same dispatch path `app.rs` routes.

    use crate::docs::DocService;
    use crate::docs::service::{Conversion, Manifest, PictureMeta, StubDocService};
    use crate::mcp::server::{
        CopyArgs, DeleteArgs, DocumentViewArgs, DocumentizeArgs, EditArgs, ExcludePatterns,
        ExportZipArgs, ExtractArchiveArgs, ExtractTextArgs, GetImageArgs, GetTableArgs,
        GitBranchCreateArgs, GitBranchSwitchArgs, GitCheckoutFileArgs, GitCommitArgs, GlobArgs,
        GrepArgs, ListDirArgs, ListImagesArgs, ListTablesArgs, McpServer, MountOnlyArgs, MoveArgs,
        ProjectIdArgs, ReadBytesArgs, TrashRestoreArgs, WriteBytesArgs,
    };
    use crate::state::AppState;
    use crate::tools::admin::test_support::Fixture;
    use base64::Engine as _;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::{CallToolResult, ContentBlock};
    use serde_json::{Value, json};
    use std::sync::Arc;

    const ALICE: &str = "alice@test.com";
    const PROJECT: &str = "proj";

    fn text_of(r: &CallToolResult) -> String {
        r.content
            .iter()
            .find_map(|c| match c {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .expect("a text content block")
    }

    fn ok_json(r: Result<CallToolResult, rmcp::ErrorData>) -> Value {
        let r = r.expect("a tool result, not a protocol error");
        assert_ne!(r.is_error, Some(true), "expected success, got: {}", text_of(&r));
        serde_json::from_str(&text_of(&r)).expect("the tool result is JSON")
    }

    fn err_text(r: Result<CallToolResult, rmcp::ErrorData>) -> String {
        let r = r.expect("a tool result, not a protocol error");
        assert_eq!(r.is_error, Some(true), "expected an error, got: {}", text_of(&r));
        text_of(&r)
    }

    /// A minimal OOXML workbook: one worksheet per `(name, rows)`, every cell
    /// an inline string, built in memory like the extraction unit tests do.
    fn xlsx(sheets: &[(&str, Vec<Vec<String>>)]) -> Vec<u8> {
        let mut workbook = String::from(r#"<workbook xmlns:r="r"><sheets>"#);
        let mut rels = String::from(r#"<Relationships xmlns="x">"#);
        let mut parts: Vec<(String, String)> = Vec::new();
        for (i, (name, rows)) in sheets.iter().enumerate() {
            let n = i + 1;
            workbook.push_str(&format!(r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#));
            rels.push_str(&format!(
                r#"<Relationship Id="rId{n}" Type="http://x/worksheet" Target="worksheets/sheet{n}.xml"/>"#
            ));
            let mut sheet = String::from("<worksheet><sheetData>");
            for row in rows {
                sheet.push_str("<row>");
                for cell in row {
                    sheet.push_str(&format!(r#"<c t="inlineStr"><is><t>{cell}</t></is></c>"#));
                }
                sheet.push_str("</row>");
            }
            sheet.push_str("</sheetData></worksheet>");
            parts.push((format!("xl/worksheets/sheet{n}.xml"), sheet));
        }
        workbook.push_str("</sheets></workbook>");
        rels.push_str("</Relationships>");
        parts.push(("xl/workbook.xml".into(), workbook));
        parts.push(("xl/_rels/workbook.xml.rels".into(), rels));

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, xml) in &parts {
            zip.start_file(name.as_str(), opts).unwrap();
            std::io::Write::write_all(&mut zip, xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn grid(header: &[&str], rows: usize, prefix: &str) -> Vec<Vec<String>> {
        let mut out = vec![header.iter().map(|h| (*h).to_string()).collect::<Vec<_>>()];
        for r in 0..rows {
            out.push(header.iter().map(|h| format!("{prefix}{h}{r}")).collect());
        }
        out
    }

    fn extract_args(path: &str, max_chars: i64) -> ExtractTextArgs {
        ExtractTextArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            max_chars,
            preview_chars: 4_000,
            ocr: false,
            refresh: false,
        }
    }

    fn list_args(path: &str) -> ListTablesArgs {
        ListTablesArgs { mount_id: PROJECT.into(), path: path.into(), marker: None }
    }

    async fn setup(quota: Option<i64>) -> (Fixture, McpServer) {
        let f = Fixture::with_config(|c| {
            if let Some(q) = quota {
                c.safety.write_quota_bytes = q;
            }
        })
        .await;
        f.seed_project(PROJECT, ALICE).await;
        let server = McpServer::new(f.state.clone(), ALICE.to_string());
        (f, server)
    }

    async fn put(f: &Fixture, path: &str, bytes: &[u8]) {
        let client = f.state.stores.client(PROJECT).await.unwrap();
        client.write_bytes_atomic(path, bytes).await.unwrap();
    }

    /// SPEC-0019/E2E-014: a text extraction refused for the session write quota
    /// leaves no Markdown sibling and no artifacts (FR-MOD-001, FR-NEW-013).
    #[tokio::test]
    async fn spec_0019_e2e_014_quota_refused_extraction_leaves_no_sibling_and_no_tables() {
        let (f, server) = setup(Some(262_144)).await;
        let book = xlsx(&[
            ("Q1", grid(&["item", "amount"], 60, "q1")),
            ("Q2", grid(&["item", "amount"], 60, "q2")),
        ]);
        put(&f, "/budget.xlsx", &book).await;
        // Alice has already written 262 000 bytes this session.
        f.state.safety.charge_write(ALICE, PROJECT, 262_000).unwrap();

        let err = err_text(
            server.fs_extract_text(Parameters(extract_args("/budget.xlsx", 200_000))).await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");

        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(
            !client.exists("/budget.md").await.unwrap(),
            "a refused extraction writes no sibling"
        );

        let err = err_text(server.fs_list_tables(Parameters(list_args("/budget.xlsx"))).await);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// SPEC-0019/E2E-101: Excel tables are captioned with their sheet names, in
    /// workbook order, labelled exact (FR-NEW-017, FR-NEW-005).
    #[tokio::test]
    async fn spec_0019_e2e_101_excel_tables_take_their_sheet_names_as_captions() {
        let (f, server) = setup(None).await;
        let book = xlsx(&[
            ("Q1", grid(&["item", "amount"], 2, "a")),
            ("Q2", grid(&["item", "amount", "note"], 3, "b")),
        ]);
        put(&f, "/budget.xlsx", &book).await;

        ok_json(server.fs_extract_text(Parameters(extract_args("/budget.xlsx", 200_000))).await);
        let out = ok_json(server.fs_list_tables(Parameters(list_args("/budget.xlsx"))).await);

        assert_eq!(
            out,
            json!({
                "tables": [
                    {"id": "table-1", "caption": "Q1", "rows": 2, "columns": 2, "csv_quality": "CSV quality: exact"},
                    {"id": "table-2", "caption": "Q2", "rows": 3, "columns": 3, "csv_quality": "CSV quality: exact"},
                ],
                "marker": null,
            })
        );

        // The independent test: a rewrite makes the document not extracted.
        put(&f, "/budget.xlsx", &book).await;
        let err = err_text(server.fs_list_tables(Parameters(list_args("/budget.xlsx"))).await);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// The text `two.xlsx` extracts to, so a test can place its character limit
    /// inside a given sheet's table.
    fn two_xlsx() -> Vec<u8> {
        xlsx(&[("A", grid(&["x", "y"], 2, "a")), ("B", grid(&["x", "y"], 2, "b"))])
    }

    /// SPEC-0019/E2E-130: a limit cutting inside sheet B's table keeps exactly
    /// table-1, caption A (FR-NEW-034).
    #[tokio::test]
    async fn spec_0019_e2e_130_a_cut_inside_the_second_table_keeps_only_the_first() {
        let (f, server) = setup(None).await;
        put(&f, "/two.xlsx", &two_xlsx()).await;
        let full =
            ok_json(server.fs_extract_text(Parameters(extract_args("/two.xlsx", 200_000))).await);
        let text = full["preview"].as_str().unwrap().to_string();
        // Inside B's table: just past its header line.
        let cut = text[..text.find("| bx0").expect("B's first data row")].chars().count() as i64;

        let mut args = extract_args("/two.xlsx", cut);
        args.refresh = true;
        let out = ok_json(server.fs_extract_text(Parameters(args)).await);
        assert_eq!(out["truncated"], json!(true));

        let out = ok_json(server.fs_list_tables(Parameters(list_args("/two.xlsx"))).await);
        let tables = out["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 1, "got: {out}");
        assert_eq!(tables[0]["id"], "table-1");
        assert_eq!(tables[0]["caption"], "A");
        assert_eq!(tables[0]["rows"], 2, "a kept table keeps every row");
    }

    /// SPEC-0019/E2E-131: a limit cutting inside sheet A's table keeps no table
    /// (FR-NEW-034).
    #[tokio::test]
    async fn spec_0019_e2e_131_a_cut_inside_the_first_table_keeps_none() {
        let (f, server) = setup(None).await;
        put(&f, "/two.xlsx", &two_xlsx()).await;
        let full =
            ok_json(server.fs_extract_text(Parameters(extract_args("/two.xlsx", 200_000))).await);
        let text = full["preview"].as_str().unwrap().to_string();
        let cut = text[..text.find("| ax1").expect("A's second data row")].chars().count() as i64;

        let mut args = extract_args("/two.xlsx", cut);
        args.refresh = true;
        ok_json(server.fs_extract_text(Parameters(args)).await);

        let out = ok_json(server.fs_list_tables(Parameters(list_args("/two.xlsx"))).await);
        assert_eq!(out, json!({"tables": [], "marker": null}));
    }

    /// FR-NEW-023: a document never converted has no artifacts; FR-NEW-028: a
    /// path holding no file answers `not a file`, checked first (FR-NEW-031).
    #[tokio::test]
    async fn never_converted_and_missing_documents_are_refused_in_order() {
        let (f, server) = setup(None).await;
        put(&f, "/budget.xlsx", &two_xlsx()).await;
        let err = err_text(server.fs_list_tables(Parameters(list_args("/budget.xlsx"))).await);
        assert!(
            err.contains("ERR_NOT_FOUND") && err.contains("No artifacts: document not extracted"),
            "got: {err}"
        );

        let mut args = list_args("/missing.xlsx");
        args.marker = Some("garbage".into());
        let err = err_text(server.fs_list_tables(Parameters(args)).await);
        assert!(err.contains("not a file: /missing.xlsx"), "got: {err}");

        let mut args = list_args("/budget.xlsx");
        args.marker = Some("garbage".into());
        let err = err_text(server.fs_list_tables(Parameters(args)).await);
        assert!(
            err.contains("ERR_INVALID_ARGUMENT")
                && err.contains("Invalid continuation marker: garbage"),
            "got: {err}"
        );
    }

    /// FR-NEW-005 paging: 100 per page, a marker only when more remain, scoped
    /// to the document that issued it (DEC-010).
    #[tokio::test]
    async fn tables_page_by_one_hundred_with_a_scoped_marker() {
        let (f, server) = setup(None).await;
        // A Word document is not needed: one sheet per table, 101 sheets.
        let sheets: Vec<(String, Vec<Vec<String>>)> =
            (0..101).map(|i| (format!("S{i}"), grid(&["k"], 1, "v"))).collect();
        let refs: Vec<(&str, Vec<Vec<String>>)> =
            sheets.iter().map(|(n, g)| (n.as_str(), g.clone())).collect();
        put(&f, "/many.xlsx", &xlsx(&refs)).await;
        put(&f, "/other.xlsx", &two_xlsx()).await;
        ok_json(server.fs_extract_text(Parameters(extract_args("/many.xlsx", 200_000))).await);

        let first = ok_json(server.fs_list_tables(Parameters(list_args("/many.xlsx"))).await);
        assert_eq!(first["tables"].as_array().unwrap().len(), 100);
        let marker = first["marker"].as_str().expect("a marker when more remain").to_string();

        let mut args = list_args("/many.xlsx");
        args.marker = Some(marker.clone());
        let second = ok_json(server.fs_list_tables(Parameters(args)).await);
        assert_eq!(
            second["tables"],
            json!([{"id": "table-101", "caption": "S100", "rows": 1, "columns": 1, "csv_quality": "CSV quality: exact"}])
        );
        assert_eq!(second["marker"], Value::Null);

        let mut args = list_args("/other.xlsx");
        args.marker = Some(marker.clone());
        let err = err_text(server.fs_list_tables(Parameters(args)).await);
        assert!(err.contains(&format!("Invalid continuation marker: {marker}")), "got: {err}");
    }

    // ── converter capture (US-0002) ─────────────────────────────────────────

    /// The session limit the quota tests pin (256 KiB).
    const QUOTA: i64 = 262_144;

    /// A server whose converter is `doc`, sharing the fixture's stores and
    /// session accounting, so `put` and the quota see the same state.
    fn with_doc(f: &Fixture, doc: Option<Arc<dyn DocService>>) -> McpServer {
        with_doc_as(f, doc, ALICE)
    }

    /// [`with_doc`] acting as `person`.
    fn with_doc_as(f: &Fixture, doc: Option<Arc<dyn DocService>>, person: &str) -> McpServer {
        let s = &f.state;
        let state = AppState {
            config: s.config.clone(),
            admin: s.admin.clone(),
            stores: s.stores.clone(),
            safety: s.safety.clone(),
            identity: s.identity.clone(),
            editors: s.editors.clone(),
            doc_service: doc,
            search: None,
        };
        McpServer::new(Arc::new(state), person.to_string())
    }

    /// A project, a document at `path`, and a server whose fake converter
    /// answers `markdown` for every document.
    async fn converting(path: &str, markdown: &str, quota: Option<i64>) -> (Fixture, McpServer) {
        let (f, _) = setup(quota).await;
        put(&f, path, b"%PDF-1.4 source").await;
        let server = with_doc(&f, Some(Arc::new(StubDocService::ok(markdown))));
        (f, server)
    }

    fn documentize_args(path: &str, overwrite: bool) -> DocumentizeArgs {
        DocumentizeArgs { mount_id: PROJECT.into(), path: path.into(), overwrite }
    }

    fn write_args(path: &str, bytes: &[u8], documentation: bool) -> WriteBytesArgs {
        WriteBytesArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            overwrite: false,
            create_parents: true,
            trigger_documentation_service: documentation,
        }
    }

    async fn convert(server: &McpServer, path: &str) -> Value {
        ok_json(server.fs_documentize(Parameters(documentize_args(path, false))).await)
    }

    async fn tables(server: &McpServer, path: &str) -> Value {
        ok_json(server.fs_list_tables(Parameters(list_args(path))).await)
    }

    fn ids(list: &Value) -> Vec<String> {
        list["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// The independent test of US-0002: a converter table is kept, reported and
    /// listed, read back from the store.
    #[tokio::test]
    async fn us_0002_a_converter_table_is_reported_and_listed() {
        let (_f, server) =
            converting("/t.pdf", "| a | b |\n| --- | --- |\n| 1 | 2 |\n", None).await;
        let out = convert(&server, "/t.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 1 tables");
        // Every key of today is unchanged (FR-NEW-030).
        assert_eq!(out["path"], "/t.pdf");
        assert_eq!(out["md_path"], "/t.md");
        assert_eq!(out["overwritten"], false);
        assert_eq!(
            tables(&server, "/t.pdf").await,
            json!({
                "tables": [{"id": "table-1", "caption": "", "rows": 1, "columns": 2, "csv_quality": "CSV quality: approximate"}],
                "marker": null,
            })
        );
    }

    /// SPEC-0019/E2E-006: a malformed table is reported, not kept, and keeps its
    /// id unused (FR-NEW-015, FR-NEW-001).
    #[tokio::test]
    async fn spec_0019_e2e_006_a_malformed_table_is_reported_and_skipped() {
        let md = "| a |\n| --- |\n| 1 |\n\n| a | b |\n| --- | --- | --- |\n| 1 | 2 |\n\n| c |\n| --- |\n| 3 |\n";
        let (_f, server) = converting("/three.pdf", md, None).await;
        let out = convert(&server, "/three.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 2 tables\ntable-2: malformed table");
        assert_eq!(ids(&tables(&server, "/three.pdf").await), vec!["table-1", "table-3"]);
    }

    /// SPEC-0019/E2E-010: a format the converter refuses keeps no artifact.
    #[tokio::test]
    async fn spec_0019_e2e_010_a_refused_format_keeps_no_artifact() {
        let (f, _) = setup(None).await;
        put(&f, "/budget.xlsx", &two_xlsx()).await;
        let pdf_only = StubDocService::ok("| a |\n| --- |\n").with_extensions(&[".pdf"]);
        let server = with_doc(&f, Some(Arc::new(pdf_only)));
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/budget.xlsx", false))).await,
        );
        assert!(
            err.contains("'/budget.xlsx' cannot be documented, the document service accepts: .pdf"),
            "got: {err}"
        );
        let err = err_text(server.fs_list_tables(Parameters(list_args("/budget.xlsx"))).await);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// SPEC-0019/E2E-011: no converter configured keeps no artifact.
    #[tokio::test]
    async fn spec_0019_e2e_011_no_converter_keeps_no_artifact() {
        let (f, _) = setup(None).await;
        put(&f, "/budget.xlsx", &two_xlsx()).await;
        let server = with_doc(&f, None);
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/budget.xlsx", false))).await,
        );
        assert!(err.contains("document service is not configured"), "got: {err}");
        let err = err_text(server.fs_list_tables(Parameters(list_args("/budget.xlsx"))).await);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// SPEC-0019/E2E-020: the Markdown sibling is byte identical to what the
    /// converter produced, before and after a re-conversion (FR-NEW-022).
    #[tokio::test]
    async fn spec_0019_e2e_020_the_sibling_bytes_are_unchanged() {
        let md = "# Report\n\nTable 1: Costs\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (f, server) = converting("/report.pdf", md, None).await;
        convert(&server, "/report.pdf").await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let reference = client.read_bytes("/report.md").await.unwrap();
        assert_eq!(reference, md.as_bytes());

        let out =
            ok_json(server.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await);
        assert_eq!(out["overwritten"], true);
        assert_eq!(client.read_bytes("/report.md").await.unwrap(), reference);
    }

    /// SPEC-0019/E2E-021: a 200 000 byte conversion fits, the next 70 000 byte
    /// write does not (FR-NEW-013).
    #[tokio::test]
    async fn spec_0019_e2e_021_the_conversion_is_charged_to_the_session_quota() {
        // No table: the set is empty, sibling and conversion text 100 000 each.
        let md = "z".repeat(100_000);
        let (_f, server) = converting("/report.pdf", &md, Some(QUOTA)).await;
        let out = convert(&server, "/report.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");

        let err = err_text(
            server
                .fs_write_bytes(Parameters(write_args("/notes.txt", &[b'n'; 70_000], false)))
                .await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
    }

    /// SPEC-0019/E2E-043: converter tables are approximate, built-in ones exact.
    #[tokio::test]
    async fn spec_0019_e2e_043_csv_quality_follows_the_conversion() {
        let (f, server) = converting("/report.pdf", "| a |\n| --- |\n| 1 |\n", None).await;
        put(&f, "/budget.xlsx", &two_xlsx()).await;
        convert(&server, "/report.pdf").await;
        ok_json(server.fs_extract_text(Parameters(extract_args("/budget.xlsx", 200_000))).await);

        let quality = |list: &Value| -> Vec<String> {
            list["tables"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["csv_quality"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            quality(&tables(&server, "/report.pdf").await),
            vec!["CSV quality: approximate"]
        );
        assert_eq!(
            quality(&tables(&server, "/budget.xlsx").await),
            vec!["CSV quality: exact", "CSV quality: exact"]
        );
    }

    /// SPEC-0019/E2E-045: 101 converter tables page as 100 then 1 (FR-NEW-005).
    #[tokio::test]
    async fn spec_0019_e2e_045_converter_tables_page_by_one_hundred() {
        let md: String = (1..=101).map(|i| format!("| k{i} |\n| --- |\n| v |\n\n")).collect();
        let (_f, server) = converting("/big.pdf", &md, None).await;
        assert_eq!(convert(&server, "/big.pdf").await["artifacts_report"], "0 images, 101 tables");

        let first = tables(&server, "/big.pdf").await;
        let expected: Vec<String> = (1..=100).map(|i| format!("table-{i}")).collect();
        assert_eq!(ids(&first), expected);
        let mut args = list_args("/big.pdf");
        args.marker = Some(first["marker"].as_str().expect("a marker").to_string());
        let second = ok_json(server.fs_list_tables(Parameters(args)).await);
        assert_eq!(ids(&second), vec!["table-101"]);
        assert_eq!(second["marker"], Value::Null);
    }

    /// SPEC-0019/E2E-106: no converter, a documented upload writes nothing.
    #[tokio::test]
    async fn spec_0019_e2e_106_a_documented_upload_without_converter_writes_nothing() {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, None);
        let err = err_text(
            server.fs_write_bytes(Parameters(write_args("/report.pdf", b"%PDF", true))).await,
        );
        assert!(err.contains("document service is not configured"), "got: {err}");
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/report.pdf").await.unwrap());
    }

    /// SPEC-0019/E2E-110: a refused format, a documented upload writes nothing.
    #[tokio::test]
    async fn spec_0019_e2e_110_a_documented_upload_of_a_refused_format_writes_nothing() {
        let (f, _) = setup(None).await;
        let pdf_only = StubDocService::ok("x").with_extensions(&[".pdf"]);
        let server = with_doc(&f, Some(Arc::new(pdf_only)));
        let err = err_text(
            server.fs_write_bytes(Parameters(write_args("/budget.xlsx", &two_xlsx(), true))).await,
        );
        assert!(
            err.contains("'/budget.xlsx' cannot be documented, the document service accepts: .pdf"),
            "got: {err}"
        );
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/budget.xlsx").await.unwrap());
    }

    /// SPEC-0019/E2E-128: a line other than `Table <n>: <text>` is no caption.
    #[tokio::test]
    async fn spec_0019_e2e_128_a_plain_line_before_a_table_is_no_caption() {
        let md = "Summary\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (_f, server) = converting("/nocap.pdf", md, None).await;
        convert(&server, "/nocap.pdf").await;
        let list = tables(&server, "/nocap.pdf").await;
        assert_eq!(ids(&list), vec!["table-1"]);
        assert_eq!(list["tables"][0]["caption"], "");
    }

    /// SPEC-0019/E2E-145: a table inside a fenced code block is no table.
    #[tokio::test]
    async fn spec_0019_e2e_145_a_fenced_table_is_no_table() {
        let md = "```\n| a | b |\n| --- | --- |\n```\n";
        let (_f, server) = converting("/code.pdf", md, None).await;
        assert_eq!(convert(&server, "/code.pdf").await["artifacts_report"], "0 images, 0 tables");
        assert_eq!(tables(&server, "/code.pdf").await, json!({"tables": [], "marker": null}));
    }

    /// SPEC-0019/E2E-146: two pipe lines without a separator are no table.
    #[tokio::test]
    async fn spec_0019_e2e_146_no_separator_no_table() {
        let (_f, server) = converting("/sep.pdf", "| a | b |\n| c | d |\n", None).await;
        assert_eq!(convert(&server, "/sep.pdf").await["artifacts_report"], "0 images, 0 tables");
    }

    /// SPEC-0019/E2E-148: lines not starting with `|` are never a table.
    #[tokio::test]
    async fn spec_0019_e2e_148_lines_without_a_leading_pipe_are_no_table() {
        let (_f, server) = converting("/bare.pdf", "a | b\n--- | ---\n1 | 2\n", None).await;
        assert_eq!(convert(&server, "/bare.pdf").await["artifacts_report"], "0 images, 0 tables");
    }

    /// SPEC-0019/E2E-149: an escaped pipe makes the separator's count differ.
    #[tokio::test]
    async fn spec_0019_e2e_149_an_escaped_pipe_makes_a_malformed_table() {
        let (_f, server) =
            converting("/esc2.pdf", "| a\\|b | c |\n| --- | --- | --- |\n", None).await;
        assert_eq!(
            convert(&server, "/esc2.pdf").await["artifacts_report"],
            "0 images, 0 tables\ntable-1: malformed table"
        );
    }

    /// SPEC-0019/E2E-150: rows without a trailing pipe still split into cells.
    #[tokio::test]
    async fn spec_0019_e2e_150_rows_without_a_trailing_pipe_are_read() {
        let (_f, server) = converting("/trail.pdf", "| a | b\n| --- | ---\n| 1 | 2\n", None).await;
        convert(&server, "/trail.pdf").await;
        let list = tables(&server, "/trail.pdf").await;
        assert_eq!(list["tables"][0]["rows"], 1);
        assert_eq!(list["tables"][0]["columns"], 2);
    }

    /// SPEC-0019/E2E-156: set 100 000, sibling 80 000 and conversion text
    /// 80 000 bytes are charged together, so a 10 000 byte write is refused
    /// (FR-NEW-013, DEC-005).
    #[tokio::test]
    async fn spec_0019_e2e_156_set_sibling_and_text_are_charged_together() {
        // One table: CSV `h\n<cell>` and Markdown `| h |\n| --- |\n| <cell> |`
        // weigh 2 x 49 990 + 20 = 100 000 bytes.
        let table = format!("| h |\n| --- |\n| {} |", "x".repeat(49_990));
        let md = format!("{table}\n\n{}", "y".repeat(80_000 - table.len() - 2));
        assert_eq!(md.len(), 80_000);
        let (_f, server) = converting("/t.pdf", &md, Some(QUOTA)).await;
        assert_eq!(convert(&server, "/t.pdf").await["artifacts_report"], "0 images, 1 tables");

        let err = err_text(
            server.fs_write_bytes(Parameters(write_args("/n.txt", &[b'n'; 10_000], false))).await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        // Exactly 260 000 were charged: what remains still fits.
        ok_json(
            server.fs_write_bytes(Parameters(write_args("/m.txt", &[b'm'; 2_144], false))).await,
        );
    }

    /// FR-NEW-013 on the converter path: a re-conversion over the quota is
    /// refused, charges nothing, and keeps the previous sibling and set.
    #[tokio::test]
    async fn a_reconversion_over_the_quota_charges_nothing_and_keeps_the_previous_set() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        let (f, server) = converting("/t.pdf", md, Some(QUOTA)).await;
        convert(&server, "/t.pdf").await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let sibling = client.read_bytes("/t.md").await.unwrap();
        let set = tables(&server, "/t.pdf").await;
        // Leave 10 bytes: less than the sibling alone.
        let used = f.state.safety.bytes_written(ALICE, PROJECT);
        f.state.safety.charge_write(ALICE, PROJECT, QUOTA - 10 - used).unwrap();

        let err =
            err_text(server.fs_documentize(Parameters(documentize_args("/t.pdf", true))).await);
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        assert_eq!(f.state.safety.bytes_written(ALICE, PROJECT), QUOTA - 10);
        assert_eq!(client.read_bytes("/t.md").await.unwrap(), sibling);
        assert_eq!(tables(&server, "/t.pdf").await, set);
        // Nothing was charged: the 10 remaining bytes still fit.
        ok_json(server.fs_write_bytes(Parameters(write_args("/n.txt", &[b'n'; 10], false))).await);
    }

    /// FR-NEW-013 and FR-NEW-014: a documented upload whose conversion is over
    /// the quota keeps the source, charged, and no sibling and no set.
    #[tokio::test]
    async fn a_documented_upload_over_the_quota_keeps_the_source_only() {
        let (f, _) = setup(Some(QUOTA)).await;
        let md = format!("| a |\n| --- |\n| 1 |\n\n{}", "z".repeat(1_000));
        let server = with_doc(&f, Some(Arc::new(StubDocService::ok(&md))));
        f.state.safety.charge_write(ALICE, PROJECT, QUOTA - 500).unwrap();

        let out = ok_json(
            server.fs_write_bytes(Parameters(write_args("/deck.pdf", &[b'p'; 100], true))).await,
        );
        let message = out["documentation"]["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("session write quota of 262144 bytes exceeded"), "got: {out}");
        assert_eq!(f.state.safety.bytes_written(ALICE, PROJECT), QUOTA - 400);
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert_eq!(client.read_bytes("/deck.pdf").await.unwrap(), vec![b'p'; 100]);
        assert!(!client.exists("/deck.md").await.unwrap(), "no sibling");
        let err = err_text(server.fs_list_tables(Parameters(list_args("/deck.pdf"))).await);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// FR-MOD-002: a documented upload carries the report inside its
    /// documentation outcome, every other key unchanged.
    #[tokio::test]
    async fn a_documented_upload_reports_inside_its_documentation() {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, Some(Arc::new(StubDocService::ok("| a |\n| --- |\n"))));
        let out = ok_json(
            server.fs_write_bytes(Parameters(write_args("/deck.pdf", b"%PDF", true))).await,
        );
        assert_eq!(
            out["documentation"],
            json!({"md_path": "/deck.md", "bytes_written": 14, "artifacts_report": "0 images, 1 tables"})
        );
        assert_eq!(tables(&server, "/deck.pdf").await["tables"][0]["rows"], 0);
    }

    /// FR-MOD-002: a text extraction that rewrites the sibling reports; one
    /// answered from the existing sibling does not (FR-NEW-030).
    #[tokio::test]
    async fn a_text_extraction_reports_only_when_it_rewrites_the_sibling() {
        let (f, server) = setup(None).await;
        put(&f, "/two.xlsx", &two_xlsx()).await;
        let fresh =
            ok_json(server.fs_extract_text(Parameters(extract_args("/two.xlsx", 200_000))).await);
        assert_eq!(fresh["artifacts_report"], "0 images, 2 tables");
        let cached =
            ok_json(server.fs_extract_text(Parameters(extract_args("/two.xlsx", 200_000))).await);
        assert_eq!(cached["cached"], true);
        assert!(cached.get("artifacts_report").is_none(), "got: {cached}");
    }

    // ── get a table (US-0003) ───────────────────────────────────────────────

    /// The conversion text of the shared `report.pdf` (spec section 9).
    const REPORT_MD: &str = "# Report\n\nTable 1: Q1 sales\n\n\
        | Region | Jan | Feb | Mar |\n| --- | --- | --- | --- |\n\
        | North | 10 | 11 | 12 |\n| South | 20 | 21 | 22 |\n| West | 30 | 31 | 32 |\n\n\
        Table 2: Costs\n\n| Item | EUR |\n| --- | --- |\n| Rent | 900 |\n| Power | 120 |\n";

    async fn report() -> (Fixture, McpServer) {
        let (f, server) = converting("/report.pdf", REPORT_MD, None).await;
        convert(&server, "/report.pdf").await;
        (f, server)
    }

    fn get_args(path: &str, id: &str, format: Option<&str>) -> GetTableArgs {
        GetTableArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            id: id.into(),
            format: format.map(str::to_string),
        }
    }

    async fn get(server: &McpServer, path: &str, id: &str, format: Option<&str>) -> Value {
        ok_json(server.fs_get_table(Parameters(get_args(path, id, format))).await)
    }

    async fn content(server: &McpServer, path: &str, id: &str, format: Option<&str>) -> String {
        get(server, path, id, format).await["content"].as_str().expect("content").to_string()
    }

    /// Extract `path` with the built-in conversion (no converter needed).
    async fn extracted(f: &Fixture, server: &McpServer, path: &str, bytes: &[u8]) {
        put(f, path, bytes).await;
        ok_json(server.fs_extract_text(Parameters(extract_args(path, 200_000))).await);
    }

    /// An RFC 4180 reader, independent of the renderer, to read a CSV back.
    fn read_csv(text: &str) -> Vec<Vec<String>> {
        let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
        let mut chars = text.chars().peekable();
        let mut quoted = false;
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                (true, '"') => quoted = false,
                (true, c) => field.push(c),
                (false, '"') => quoted = true,
                (false, ',') => row.push(std::mem::take(&mut field)),
                (false, '\n') => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                (false, c) => field.push(c),
            }
        }
        row.push(field);
        rows.push(row);
        rows
    }

    /// SPEC-0019/E2E-034: the two converted tables, captioned, approximate.
    #[tokio::test]
    async fn spec_0019_e2e_034_report_tables_are_listed_approximate() {
        let (_f, server) = report().await;
        assert_eq!(
            tables(&server, "/report.pdf").await,
            json!({
                "tables": [
                    {"id": "table-1", "caption": "Q1 sales", "rows": 3, "columns": 4, "csv_quality": "CSV quality: approximate"},
                    {"id": "table-2", "caption": "Costs", "rows": 2, "columns": 2, "csv_quality": "CSV quality: approximate"},
                ],
                "marker": null,
            })
        );
    }

    /// SPEC-0019/E2E-035: no format means Markdown, byte exact (FR-NEW-006).
    #[tokio::test]
    async fn spec_0019_e2e_035_no_format_returns_markdown() {
        let (_f, server) = report().await;
        let out = get(&server, "/report.pdf", "table-1", None).await;
        assert_eq!(
            out,
            json!({
                "id": "table-1",
                "format": "markdown",
                "content": "| Region | Jan | Feb | Mar |\n| --- | --- | --- | --- |\n\
                            | North | 10 | 11 | 12 |\n| South | 20 | 21 | 22 |\n| West | 30 | 31 | 32 |",
            })
        );
    }

    /// SPEC-0019/E2E-036: CSV is a header and 3 rows of 4 fields.
    #[tokio::test]
    async fn spec_0019_e2e_036_csv_has_four_lines_of_four_fields() {
        let (_f, server) = report().await;
        let out = get(&server, "/report.pdf", "table-1", Some("csv")).await;
        assert_eq!(out["id"], "table-1");
        assert_eq!(out["format"], "csv");
        let csv = out["content"].as_str().unwrap();
        let lines: Vec<&str> = csv.split('\n').collect();
        assert_eq!(lines.len(), 4, "got: {csv:?}");
        assert!(lines.iter().all(|l| l.split(',').count() == 4), "got: {csv:?}");
        assert_eq!(lines[0], "Region,Jan,Feb,Mar");
        assert_eq!(lines[3], "West,30,31,32");
    }

    /// SPEC-0019/E2E-037: an id the set does not hold is not found.
    #[tokio::test]
    async fn spec_0019_e2e_037_an_unknown_table_is_not_found() {
        let (_f, server) = report().await;
        let err = err_text(
            server.fs_get_table(Parameters(get_args("/report.pdf", "table-9", None))).await,
        );
        assert!(err.contains("ERR_NOT_FOUND") && err.contains("Not found: table-9"), "got: {err}");
    }

    async fn refused_format(format: &str) {
        let (_f, server) = report().await;
        let err = err_text(
            server.fs_get_table(Parameters(get_args("/report.pdf", "table-1", Some(format)))).await,
        );
        assert!(
            err.contains("ERR_INVALID_ARGUMENT")
                && err.contains(&format!("Unsupported format: {format} (use markdown or csv)")),
            "got: {err}"
        );
        assert!(!err.contains("Region"), "no table content on a refusal: {err}");
    }

    /// SPEC-0019/E2E-039: `xml` is refused.
    #[tokio::test]
    async fn spec_0019_e2e_039_xml_is_refused() {
        refused_format("xml").await;
    }

    /// SPEC-0019/E2E-040: the format is case sensitive, `CSV` is refused.
    #[tokio::test]
    async fn spec_0019_e2e_040_upper_case_csv_is_refused() {
        refused_format("CSV").await;
    }

    /// SPEC-0019/E2E-041: `md` is no alias of `markdown`.
    #[tokio::test]
    async fn spec_0019_e2e_041_md_is_refused() {
        refused_format("md").await;
    }

    /// SPEC-0019/E2E-044: pipes, quotes, commas, accents and emoji survive
    /// both formats; the CSV reads back to the original cells.
    #[tokio::test]
    async fn spec_0019_e2e_044_special_cells_survive_both_formats() {
        let (f, server) = setup(None).await;
        let original = ["a|b", "Dupont, \"Jr\"", "Zoë 🚀"];
        let rows = vec![
            vec!["h1".to_string(), "h2".into(), "h3".into()],
            original.iter().map(|c| (*c).to_string()).collect(),
        ];
        extracted(&f, &server, "/names.xlsx", &xlsx(&[("Names", rows.clone())])).await;

        let md = content(&server, "/names.xlsx", "table-1", Some("markdown")).await;
        let last = md.lines().last().unwrap();
        assert!(last.contains("a\\|b"), "got: {md:?}");
        assert_eq!(last, "| a\\|b | Dupont, \"Jr\" | Zoë 🚀 |");
        assert_eq!(last.replace("\\|", "").matches('|').count(), 4, "3 columns: {last}");

        let csv = content(&server, "/names.xlsx", "table-1", Some("csv")).await;
        assert!(csv.contains("\"Dupont, \"\"Jr\"\"\""), "got: {csv:?}");
        assert!(csv.contains("Zoë 🚀"), "got: {csv:?}");
        assert_eq!(read_csv(&csv), rows);
    }

    /// The `empty.xlsx` of the spec: header `A`, `B`, 0 data rows.
    fn empty_xlsx() -> Vec<u8> {
        xlsx(&[("Empty", vec![vec!["A".to_string(), "B".to_string()]])])
    }

    /// SPEC-0019/E2E-046: a header only table lists 0 rows, CSV is the header.
    #[tokio::test]
    async fn spec_0019_e2e_046_a_header_only_table_is_its_header_line_in_csv() {
        let (f, server) = setup(None).await;
        extracted(&f, &server, "/empty.xlsx", &empty_xlsx()).await;
        assert_eq!(tables(&server, "/empty.xlsx").await["tables"][0]["rows"], 0);
        assert_eq!(content(&server, "/empty.xlsx", "table-1", Some("csv")).await, "A,B");
    }

    /// SPEC-0019/E2E-140: a header only table is header and separator in
    /// Markdown.
    #[tokio::test]
    async fn spec_0019_e2e_140_a_header_only_table_is_two_markdown_lines() {
        let (f, server) = setup(None).await;
        extracted(&f, &server, "/empty.xlsx", &empty_xlsx()).await;
        assert_eq!(
            content(&server, "/empty.xlsx", "table-1", Some("markdown")).await,
            "| A | B |\n| --- | --- |"
        );
    }

    /// SPEC-0019/E2E-047: an Excel table past the 400 row sibling cap keeps
    /// every row in its CSV.
    #[tokio::test]
    async fn spec_0019_e2e_047_excel_rows_past_the_sibling_cap_are_kept() {
        let (f, server) = with_no_converter().await;
        let book = xlsx(&[("Ledger", grid(&["d", "k", "v"], 401, "r"))]);
        extracted(&f, &server, "/ledger.xlsx", &book).await;
        assert_eq!(tables(&server, "/ledger.xlsx").await["tables"][0]["rows"], 401);
        let csv = content(&server, "/ledger.xlsx", "table-1", Some("csv")).await;
        assert_eq!(csv.split('\n').count(), 402);
        assert!(csv.ends_with("rd400,rk400,rv400"), "the last row is kept");
    }

    /// A project whose server has no external converter configured.
    async fn with_no_converter() -> (Fixture, McpServer) {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, None);
        (f, server)
    }

    /// SPEC-0019/E2E-048: the second table in Markdown, byte exact.
    #[tokio::test]
    async fn spec_0019_e2e_048_costs_as_markdown() {
        let (_f, server) = report().await;
        assert_eq!(
            content(&server, "/report.pdf", "table-2", Some("markdown")).await,
            "| Item | EUR |\n| --- | --- |\n| Rent | 900 |\n| Power | 120 |"
        );
    }

    /// SPEC-0019/E2E-112: the second table in CSV, byte exact, nothing quoted.
    #[tokio::test]
    async fn spec_0019_e2e_112_costs_as_csv() {
        let (_f, server) = report().await;
        let csv = content(&server, "/report.pdf", "table-2", Some("csv")).await;
        assert_eq!(csv, "Item,EUR\nRent,900\nPower,120");
        assert!(!csv.contains('"'));
    }

    /// SPEC-0019/E2E-147: a CSV source past the 400 row sibling cap keeps every
    /// row.
    #[tokio::test]
    async fn spec_0019_e2e_147_csv_rows_past_the_sibling_cap_are_kept() {
        let (f, server) = with_no_converter().await;
        let mut source = String::from("d,k,v\n");
        for r in 0..401 {
            source.push_str(&format!("d{r},k{r},v{r}\n"));
        }
        extracted(&f, &server, "/ledger.csv", source.as_bytes()).await;
        assert_eq!(tables(&server, "/ledger.csv").await["tables"][0]["rows"], 401);
        let csv = content(&server, "/ledger.csv", "table-1", Some("csv")).await;
        assert_eq!(csv.split('\n').count(), 402);
    }

    /// SPEC-0019/E2E-154: a CR LF inside a cell is one space in Markdown and
    /// kept inside quotes in CSV.
    #[tokio::test]
    async fn spec_0019_e2e_154_a_cr_lf_cell_in_both_formats() {
        let (f, server) = with_no_converter().await;
        extracted(&f, &server, "/crlf.csv", b"k,v\nx,\"a\r\nb\"\n").await;
        let md = content(&server, "/crlf.csv", "table-1", Some("markdown")).await;
        assert_eq!(md.rsplit('\n').next().unwrap(), "| x | a b |", "got: {md:?}");
        let csv = content(&server, "/crlf.csv", "table-1", Some("csv")).await;
        assert!(csv.contains("\"a\r\nb\""), "got: {csv:?}");
    }

    /// SPEC-0019/E2E-158: a fully blank Excel row is dropped from the table.
    #[tokio::test]
    async fn spec_0019_e2e_158_a_blank_excel_row_is_dropped() {
        let (f, server) = with_no_converter().await;
        let rows = [["a", "b"], ["1", "2"], ["", ""], ["3", "4"]]
            .iter()
            .map(|r| r.iter().map(|c| (*c).to_string()).collect())
            .collect();
        extracted(&f, &server, "/gaps.xlsx", &xlsx(&[("Gaps", rows)])).await;
        assert_eq!(tables(&server, "/gaps.xlsx").await["tables"][0]["rows"], 2);
        assert_eq!(content(&server, "/gaps.xlsx", "table-1", Some("csv")).await, "a,b\n1,2\n3,4");
    }

    /// The id is matched exactly: `table-01`, `image-1` and `1` are no table.
    #[tokio::test]
    async fn only_the_canonical_table_id_is_found() {
        let (_f, server) = report().await;
        for id in ["table-01", "table-0", "image-1", "1", "table-", "table-1 "] {
            let err =
                err_text(server.fs_get_table(Parameters(get_args("/report.pdf", id, None))).await);
            assert!(err.contains(&format!("Not found: {id}")), "{id}: {err}");
        }
    }

    // ── refusal order and categories (US-0004) ──────────────────────────────

    async fn get_err(server: &McpServer, path: &str, id: &str, format: Option<&str>) -> String {
        err_text(server.fs_get_table(Parameters(get_args(path, id, format))).await)
    }

    /// SPEC-0019/E2E-038: a non member gets the same refusal as for any other
    /// operation on the project, on both table tools (FR-NEW-016).
    #[tokio::test]
    async fn spec_0019_e2e_038_a_non_member_gets_the_usual_refusal() {
        let (f, _) = report().await;
        let olga = McpServer::new(f.state.clone(), "olga@test.com".to_string());
        let usual =
            err_text(olga.fs_extract_text(Parameters(extract_args("/report.pdf", 200_000))).await);
        assert!(usual.contains("ERR_FORBIDDEN"), "got: {usual}");
        // The wrapper names the tool; the refusal after it must be identical.
        let refusal = |e: &str| e.split_once("': ").map(|(_, r)| r.to_string()).unwrap();
        let listed = err_text(olga.fs_list_tables(Parameters(list_args("/report.pdf"))).await);
        assert_eq!(refusal(&listed), refusal(&usual));
        for format in ["markdown", "csv"] {
            let got = get_err(&olga, "/report.pdf", "table-1", Some(format)).await;
            assert_eq!(refusal(&got), refusal(&usual));
        }
    }

    /// SPEC-0019/E2E-042: a never converted document has no artifacts, on both
    /// table tools (FR-NEW-023).
    #[tokio::test]
    async fn spec_0019_e2e_042_a_never_converted_document_has_no_artifacts() {
        let (_f, server) = converting("/notes.pdf", REPORT_MD, None).await;
        let listed = err_text(server.fs_list_tables(Parameters(list_args("/notes.pdf"))).await);
        let got = get_err(&server, "/notes.pdf", "table-1", Some("csv")).await;
        for err in [listed, got] {
            assert!(
                err.contains("ERR_NOT_FOUND")
                    && err.contains("No artifacts: document not extracted"),
                "got: {err}"
            );
        }
    }

    /// SPEC-0019/E2E-105: a path holding no file is `not a file` (FR-NEW-028).
    #[tokio::test]
    async fn spec_0019_e2e_105_a_missing_document_is_not_a_file() {
        let (_f, server) = setup(None).await;
        let err = err_text(server.fs_list_tables(Parameters(list_args("/ghost.pdf"))).await);
        assert!(err.contains("ERR_NOT_FOUND") && err.contains("not a file: /ghost.pdf"), "{err}");
    }

    fn assert_unsupported_xml(err: &str) {
        assert!(
            err.contains("ERR_INVALID_ARGUMENT")
                && err.contains("Unsupported format: xml (use markdown or csv)"),
            "got: {err}"
        );
    }

    /// SPEC-0019/E2E-117: the format is checked before the set (FR-NEW-031).
    #[tokio::test]
    async fn spec_0019_e2e_117_format_before_no_artifacts() {
        let (_f, server) = converting("/notes.pdf", REPORT_MD, None).await;
        assert_unsupported_xml(&get_err(&server, "/notes.pdf", "table-9", Some("xml")).await);
    }

    /// SPEC-0019/E2E-118: the format is checked before the id (FR-NEW-031).
    #[tokio::test]
    async fn spec_0019_e2e_118_format_before_unknown_id() {
        let (_f, server) = report().await;
        assert_unsupported_xml(&get_err(&server, "/report.pdf", "table-9", Some("xml")).await);
    }

    /// SPEC-0019/E2E-121: an unsupported format is an invalid argument, not a
    /// not found refusal (FR-NEW-032).
    #[tokio::test]
    async fn spec_0019_e2e_121_unsupported_format_is_an_invalid_argument() {
        let (_f, server) = report().await;
        let err = get_err(&server, "/report.pdf", "table-1", Some("xml")).await;
        assert_unsupported_xml(&err);
        assert!(!err.contains("ERR_NOT_FOUND"), "got: {err}");
    }

    /// SPEC-0019/E2E-142: an escaped pipe and Markdown emphasis survive as cell
    /// text; padding is trimmed (FR-NEW-002).
    #[tokio::test]
    async fn spec_0019_e2e_142_escaped_pipe_and_emphasis_in_both_formats() {
        let md = "| k | v |\n| --- | --- |\n|  a\\|b  | **x** |\n";
        let (_f, server) = converting("/esc.pdf", md, None).await;
        convert(&server, "/esc.pdf").await;
        assert_eq!(content(&server, "/esc.pdf", "table-1", Some("csv")).await, "k,v\na|b,**x**");
        let markdown = content(&server, "/esc.pdf", "table-1", Some("markdown")).await;
        assert_eq!(markdown.lines().last(), Some("| a\\|b | **x** |"));
    }

    // ── list and get images (US-0005) ───────────────────────────────────────

    /// One picture of a fake bundle: its target, bytes, caption and page.
    struct Pic<'a> {
        target: &'a str,
        bytes: &'a [u8],
        caption: &'a str,
        page: Option<i64>,
    }

    /// What the fake converter returns: `markdown`, every picture as a file
    /// keyed by its target, and a manifest describing them.
    fn bundle(markdown: &str, pics: &[Pic<'_>]) -> Conversion {
        let files = pics.iter().map(|p| (p.target.to_string(), p.bytes.to_vec())).collect();
        let pictures = pics
            .iter()
            .map(|p| {
                let meta = PictureMeta {
                    caption: p.caption.to_string(),
                    caption_failed: false,
                    page: p.page,
                };
                (p.target.to_string(), meta)
            })
            .collect();
        Conversion { markdown: markdown.to_string(), files, manifest: Some(Manifest { pictures }) }
    }

    /// A server whose fake converter answers `conversion`, over `f`'s stores.
    fn bundling(f: &Fixture, conversion: Conversion) -> McpServer {
        with_doc(f, Some(Arc::new(StubDocService::bundle(conversion))))
    }

    /// A project holding `path` and a server converting it to `conversion`.
    async fn converting_bundle(path: &str, conversion: Conversion) -> (Fixture, McpServer) {
        let (f, _) = setup(None).await;
        put(&f, path, b"%PDF-1.4 source").await;
        let server = bundling(&f, conversion);
        (f, server)
    }

    const FIG_1: &[u8] = b"\x89PNG\r\n\x1a\nrevenue 2024";
    const FIG_2: &[u8] = b"\x89PNG\r\n\x1a\nnetwork diagram";

    /// The shared `report.pdf` (spec section 9): 2 PNG pictures, captioned, on
    /// pages 1 and 3, around the two tables of [`REPORT_MD`].
    fn report_bundle() -> Conversion {
        let md =
            format!("![Figure](figures/f1.png)\n\n{REPORT_MD}\n![Diagram](./figures/f2.png)\n");
        bundle(
            &md,
            &[
                Pic {
                    target: "figures/f1.png",
                    bytes: FIG_1,
                    caption: "Figure 1: Revenue 2024",
                    page: Some(1),
                },
                Pic {
                    target: "figures/f2.png",
                    bytes: FIG_2,
                    caption: "Schéma réseau 🌐",
                    page: Some(3),
                },
            ],
        )
    }

    /// `report.pdf` converted through the fake converter.
    async fn report_with_images() -> (Fixture, McpServer, Value) {
        let (f, server) = converting_bundle("/report.pdf", report_bundle()).await;
        let out = convert(&server, "/report.pdf").await;
        (f, server, out)
    }

    fn list_images_args(path: &str, marker: Option<&str>) -> ListImagesArgs {
        ListImagesArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            marker: marker.map(str::to_string),
        }
    }

    fn get_image_args(path: &str, id: &str) -> GetImageArgs {
        GetImageArgs { mount_id: PROJECT.into(), path: path.into(), id: id.into() }
    }

    async fn images(server: &McpServer, path: &str, marker: Option<&str>) -> Value {
        ok_json(server.fs_list_images(Parameters(list_images_args(path, marker))).await)
    }

    async fn image(server: &McpServer, path: &str, id: &str) -> Value {
        ok_json(server.fs_get_image(Parameters(get_image_args(path, id))).await)
    }

    fn image_ids(list: &Value) -> Vec<String> {
        list["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn picture(got: &Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(got["base64"].as_str().expect("base64"))
            .expect("valid base64")
    }

    /// `n` distinct PNG pictures, one reference each, page `i` for the i-th.
    fn many_pictures(n: usize) -> Conversion {
        let targets: Vec<String> = (1..=n).map(|i| format!("p/{i}.png")).collect();
        let bytes: Vec<Vec<u8>> = (1..=n).map(|i| format!("png {i}").into_bytes()).collect();
        let md: String = targets.iter().map(|t| format!("![]({t})\n\n")).collect();
        let pics: Vec<Pic<'_>> = targets
            .iter()
            .zip(&bytes)
            .zip(1i64..)
            .map(|((t, b), page)| Pic { target: t, bytes: b, caption: "", page: Some(page) })
            .collect();
        bundle(&md, &pics)
    }

    fn numbered(prefix: &str, n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("{prefix}-{i}")).collect()
    }

    /// SPEC-0019/E2E-001: the conversion keeps both pictures and both tables,
    /// in reading order, with no missing item (FR-NEW-001).
    #[tokio::test]
    async fn spec_0019_e2e_001_report_keeps_two_images_and_two_tables() {
        let (_f, server, out) = report_with_images().await;
        assert_eq!(out["artifacts_report"], "2 images, 2 tables");
        let list = images(&server, "/report.pdf", None).await;
        assert_eq!(
            list["images"][0],
            json!({"id": "image-1", "caption": "Figure 1: Revenue 2024", "page": 1})
        );
        assert_eq!(
            list["images"][1],
            json!({"id": "image-2", "caption": "Schéma réseau 🌐", "page": 3})
        );
    }

    /// SPEC-0019/E2E-024: exactly image-1 then image-2, no marker (FR-NEW-003).
    #[tokio::test]
    async fn spec_0019_e2e_024_report_images_are_listed_in_reading_order() {
        let (_f, server, _) = report_with_images().await;
        assert_eq!(
            images(&server, "/report.pdf", None).await,
            json!({
                "images": [
                    {"id": "image-1", "caption": "Figure 1: Revenue 2024", "page": 1},
                    {"id": "image-2", "caption": "Schéma réseau 🌐", "page": 3},
                ],
                "marker": null,
            })
        );
    }

    /// SPEC-0019/E2E-025: the picture comes back byte identical, format png
    /// (FR-NEW-004).
    #[tokio::test]
    async fn spec_0019_e2e_025_get_image_returns_the_converter_bytes() {
        let (_f, server, _) = report_with_images().await;
        let got = image(&server, "/report.pdf", "image-2").await;
        assert_eq!(got["id"], "image-2");
        assert_eq!(got["format"], "png");
        assert_eq!(got["caption"], "Schéma réseau 🌐");
        assert_eq!(got["page"], 3);
        assert_eq!(picture(&got), FIG_2);
        let keys: Vec<&String> = got.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 5, "id, format, caption, page, base64: {got}");
    }

    /// SPEC-0019/E2E-026: an id the set does not hold is not found.
    #[tokio::test]
    async fn spec_0019_e2e_026_an_unknown_image_is_not_found() {
        let (_f, server, _) = report_with_images().await;
        let err = err_text(
            server.fs_get_image(Parameters(get_image_args("/report.pdf", "image-7"))).await,
        );
        assert!(err.contains("ERR_NOT_FOUND") && err.contains("Not found: image-7"), "got: {err}");
    }

    /// SPEC-0019/E2E-027: a non member gets the usual refusal on both image
    /// tools, and no caption or picture (FR-NEW-016).
    #[tokio::test]
    async fn spec_0019_e2e_027_a_non_member_gets_the_usual_refusal_on_images() {
        let (f, _, _) = report_with_images().await;
        let olga = McpServer::new(f.state.clone(), "olga@test.com".to_string());
        let usual =
            err_text(olga.fs_extract_text(Parameters(extract_args("/report.pdf", 200_000))).await);
        assert!(usual.contains("ERR_FORBIDDEN"), "got: {usual}");
        let refusal = |e: &str| e.split_once("': ").map(|(_, r)| r.to_string()).unwrap();
        let listed =
            err_text(olga.fs_list_images(Parameters(list_images_args("/report.pdf", None))).await);
        let got =
            err_text(olga.fs_get_image(Parameters(get_image_args("/report.pdf", "image-1"))).await);
        for err in [listed, got] {
            assert_eq!(refusal(&err), refusal(&usual));
            assert!(!err.contains("Revenue") && !err.contains("base64"), "got: {err}");
        }
    }

    /// SPEC-0019/E2E-028: a never converted document has no artifacts, on both
    /// image tools (FR-NEW-023).
    #[tokio::test]
    async fn spec_0019_e2e_028_a_never_converted_document_has_no_images() {
        let (_f, server) = converting_bundle("/notes.pdf", report_bundle()).await;
        let listed =
            err_text(server.fs_list_images(Parameters(list_images_args("/notes.pdf", None))).await);
        let got = err_text(
            server.fs_get_image(Parameters(get_image_args("/notes.pdf", "image-1"))).await,
        );
        for err in [listed, got] {
            assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
        }
    }

    /// SPEC-0019/E2E-029: exactly 100 images is one page and no marker.
    #[tokio::test]
    async fn spec_0019_e2e_029_one_hundred_images_fit_one_page() {
        let (_f, server) = converting_bundle("/atlas.pdf", many_pictures(100)).await;
        assert_eq!(
            convert(&server, "/atlas.pdf").await["artifacts_report"],
            "100 images, 0 tables"
        );
        let list = images(&server, "/atlas.pdf", None).await;
        assert_eq!(image_ids(&list), numbered("image", 100));
        assert_eq!(list["marker"], Value::Null);
    }

    /// SPEC-0019/E2E-030: 101 images page as 100 with a marker, then 1 without.
    #[tokio::test]
    async fn spec_0019_e2e_030_one_hundred_and_one_images_page_twice() {
        let (_f, server) = converting_bundle("/atlas2.pdf", many_pictures(101)).await;
        convert(&server, "/atlas2.pdf").await;
        let first = images(&server, "/atlas2.pdf", None).await;
        assert_eq!(image_ids(&first), numbered("image", 100));
        let marker = first["marker"].as_str().expect("a marker when more remain").to_string();
        let second = images(&server, "/atlas2.pdf", Some(&marker)).await;
        assert_eq!(
            second,
            json!({"images": [{"id": "image-101", "caption": "", "page": 101}], "marker": null})
        );
    }

    /// SPEC-0019/E2E-031: an image with no caption lists and gets an empty one.
    #[tokio::test]
    async fn spec_0019_e2e_031_an_uncaptioned_image_has_an_empty_caption() {
        let conversion = bundle(
            "Intro\n\n![](img/a.png)\n",
            &[Pic { target: "img/a.png", bytes: b"one", caption: "", page: Some(2) }],
        );
        let (_f, server) = converting_bundle("/one.pdf", conversion).await;
        convert(&server, "/one.pdf").await;
        assert_eq!(
            images(&server, "/one.pdf", None).await,
            json!({"images": [{"id": "image-1", "caption": "", "page": 2}], "marker": null})
        );
        let got = image(&server, "/one.pdf", "image-1").await;
        assert_eq!(got["caption"], "");
        assert_eq!(picture(&got), b"one");
    }

    /// SPEC-0019/E2E-032: a caption is returned exactly, pipes, quotes,
    /// accents and emoji included.
    #[tokio::test]
    async fn spec_0019_e2e_032_a_caption_is_returned_exactly() {
        let caption = "Coût | Été 😀 \"v2\"";
        let conversion = bundle(
            "![x](c.jpg)\n",
            &[Pic { target: "c.jpg", bytes: b"jpeg bytes", caption, page: Some(1) }],
        );
        let (_f, server) = converting_bundle("/cap.pdf", conversion).await;
        convert(&server, "/cap.pdf").await;
        let got = image(&server, "/cap.pdf", "image-1").await;
        assert_eq!(got["caption"], caption);
        assert_eq!(got["format"], "jpeg", "a .jpg picture is jpeg");
    }

    /// SPEC-0019/E2E-102: the same logo on two pages is two images, each with
    /// its own id and page, byte identical (FR-NEW-001).
    #[tokio::test]
    async fn spec_0019_e2e_102_each_placement_is_a_separate_image() {
        let logo = b"\x89PNG logo";
        let conversion = bundle(
            "![](logo-1.png)\n\nPage two\n\n![](logo-2.png)\n",
            &[
                Pic { target: "logo-1.png", bytes: logo, caption: "", page: Some(1) },
                Pic { target: "logo-2.png", bytes: logo, caption: "", page: Some(2) },
            ],
        );
        let (_f, server) = converting_bundle("/logo.pdf", conversion).await;
        assert_eq!(convert(&server, "/logo.pdf").await["artifacts_report"], "2 images, 0 tables");
        assert_eq!(
            images(&server, "/logo.pdf", None).await["images"],
            json!([
                {"id": "image-1", "caption": "", "page": 1},
                {"id": "image-2", "caption": "", "page": 2},
            ])
        );
        let one = picture(&image(&server, "/logo.pdf", "image-1").await);
        let two = picture(&image(&server, "/logo.pdf", "image-2").await);
        assert_eq!(one, logo);
        assert_eq!(one, two);
    }

    /// SPEC-0019/E2E-103 and E2E-122: a garbage marker is refused with its
    /// value, in the invalid argument category (FR-NEW-003, FR-NEW-032).
    #[tokio::test]
    async fn spec_0019_e2e_103_122_an_invalid_marker_is_an_invalid_argument() {
        let (_f, server, _) = report_with_images().await;
        let err = err_text(
            server.fs_list_images(Parameters(list_images_args("/report.pdf", Some("zzz")))).await,
        );
        assert!(err.contains("Invalid continuation marker: zzz"), "got: {err}");
        assert!(err.contains("ERR_INVALID_ARGUMENT") && !err.contains("ERR_NOT_FOUND"), "{err}");
    }

    /// SPEC-0019/E2E-104: a folder is not a file (FR-NEW-028).
    #[tokio::test]
    async fn spec_0019_e2e_104_a_folder_is_not_a_file() {
        let (f, server, _) = report_with_images().await;
        f.state.stores.client(PROJECT).await.unwrap().mkdir("/q1").await.unwrap();
        let err = err_text(server.fs_list_images(Parameters(list_images_args("/q1", None))).await);
        assert!(err.contains("ERR_NOT_FOUND") && err.contains("not a file: /q1"), "got: {err}");
    }

    /// SPEC-0019/E2E-115: a Word picture has an empty page, whatever the
    /// converter says (FR-NEW-001).
    #[tokio::test]
    async fn spec_0019_e2e_115_a_word_image_has_an_empty_page() {
        let conversion = bundle(
            "# Memo\n\n![chart](media/org.png)\n",
            &[Pic { target: "media/org.png", bytes: b"org", caption: "Org chart", page: Some(1) }],
        );
        let (f, _) = setup(None).await;
        put(&f, "/memo.docx", b"PK docx").await;
        let server = bundling(&f, conversion);
        convert(&server, "/memo.docx").await;
        assert_eq!(
            images(&server, "/memo.docx", None).await,
            json!({"images": [{"id": "image-1", "caption": "Org chart", "page": null}], "marker": null})
        );
        assert_eq!(image(&server, "/memo.docx", "image-1").await["page"], Value::Null);
    }

    /// SPEC-0019/E2E-120 (MCP door): a never converted document answers the
    /// not found category; the REST door is pinned in `api::dataplane`.
    #[tokio::test]
    async fn spec_0019_e2e_120_no_artifacts_is_not_found_for_the_assistant() {
        let (_f, server) = converting_bundle("/notes.pdf", report_bundle()).await;
        let err =
            err_text(server.fs_list_images(Parameters(list_images_args("/notes.pdf", None))).await);
        assert!(
            err.contains("ERR_NOT_FOUND") && err.contains("No artifacts: document not extracted"),
            "got: {err}"
        );
    }

    /// SPEC-0019/E2E-123: an images marker is refused by another document's
    /// table list, spelled as issued (DEC-010).
    #[tokio::test]
    async fn spec_0019_e2e_123_an_images_marker_is_refused_by_a_table_list() {
        let (f, _) = setup(None).await;
        put(&f, "/atlas2.pdf", b"%PDF atlas").await;
        put(&f, "/big.pdf", b"%PDF big").await;
        let atlas = bundling(&f, many_pictures(101));
        convert(&atlas, "/atlas2.pdf").await;
        let md: String = (1..=101).map(|i| format!("| k{i} |\n| --- |\n| v |\n\n")).collect();
        let big = with_doc(&f, Some(Arc::new(StubDocService::ok(md))));
        convert(&big, "/big.pdf").await;

        let marker =
            images(&atlas, "/atlas2.pdf", None).await["marker"].as_str().unwrap().to_string();
        let mut args = list_args("/big.pdf");
        args.marker = Some(marker.clone());
        let err = err_text(big.fs_list_tables(Parameters(args)).await);
        assert!(err.contains(&format!("Invalid continuation marker: {marker}")), "got: {err}");
        // Nor does it serve the images of the other document.
        let err = err_text(
            big.fs_list_images(Parameters(list_images_args("/big.pdf", Some(&marker)))).await,
        );
        assert!(err.contains(&format!("Invalid continuation marker: {marker}")), "got: {err}");
    }

    /// SPEC-0019/E2E-127: the converter's description is the image caption,
    /// the `Table 1:` line the table caption (FR-NEW-033).
    #[tokio::test]
    async fn spec_0019_e2e_127_captions_come_from_the_converter() {
        let md = "![Revenue chart](charts/rev.png)\n\nTable 1: Costs\n\n\
                  | Item | EUR |\n| --- | --- |\n| Rent | 900 |\n| Power | 120 |\n";
        let conversion = bundle(
            md,
            &[Pic {
                target: "charts/rev.png",
                bytes: b"rev",
                caption: "Revenue chart",
                page: Some(1),
            }],
        );
        let (_f, server) = converting_bundle("/cap2.pdf", conversion).await;
        assert_eq!(convert(&server, "/cap2.pdf").await["artifacts_report"], "1 images, 1 tables");
        assert_eq!(
            images(&server, "/cap2.pdf", None).await["images"][0]["caption"],
            "Revenue chart"
        );
        let list = tables(&server, "/cap2.pdf").await;
        assert_eq!(list["tables"][0]["caption"], "Costs");
        assert_eq!(list["tables"][0]["rows"], 2);
    }

    /// FR-NEW-003: a document whose conversion kept no picture lists an empty
    /// page, not an error; a marker past the end is an empty page too.
    #[tokio::test]
    async fn no_images_is_an_empty_list_and_a_marker_past_the_end_an_empty_page() {
        let (_f, server) = converting("/t.pdf", "| a |\n| --- |\n", None).await;
        convert(&server, "/t.pdf").await;
        assert_eq!(images(&server, "/t.pdf", None).await, json!({"images": [], "marker": null}));
        let past = crate::docs::artifacts::encode_marker(
            PROJECT,
            crate::docs::artifacts::ListKind::Images,
            "/t.pdf",
            500,
        );
        assert_eq!(
            images(&server, "/t.pdf", Some(&past)).await,
            json!({"images": [], "marker": null})
        );
    }

    /// The id is matched exactly: `image-01`, `table-1` and `1` are no image.
    #[tokio::test]
    async fn only_the_canonical_image_id_is_found() {
        let (_f, server, _) = report_with_images().await;
        for id in ["image-01", "image-0", "table-1", "1", "image-", "image-1 "] {
            let err =
                err_text(server.fs_get_image(Parameters(get_image_args("/report.pdf", id))).await);
            assert!(err.contains(&format!("Not found: {id}")), "{id}: {err}");
        }
    }

    /// FR-NEW-031 order on the image tools: not a file before the marker, the
    /// marker before the set.
    #[tokio::test]
    async fn image_refusals_come_in_order() {
        let (_f, server) = converting_bundle("/notes.pdf", report_bundle()).await;
        let err = err_text(
            server.fs_list_images(Parameters(list_images_args("/ghost.pdf", Some("zzz")))).await,
        );
        assert!(err.contains("not a file: /ghost.pdf"), "got: {err}");
        let err = err_text(
            server.fs_list_images(Parameters(list_images_args("/notes.pdf", Some("zzz")))).await,
        );
        assert!(err.contains("Invalid continuation marker: zzz"), "got: {err}");
        let err = err_text(
            server.fs_get_image(Parameters(get_image_args("/ghost.pdf", "image-1"))).await,
        );
        assert!(err.contains("not a file: /ghost.pdf"), "got: {err}");
    }

    /// DEC-004: each kept picture is a content addressed blob counted in
    /// `blob_refs`, so the stored bytes are the converter's, under their sha.
    #[tokio::test]
    async fn a_kept_picture_is_a_content_addressed_blob() {
        let (f, _, _) = report_with_images().await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let sha = crate::storage::VolumeClient::sha256_hex(FIG_1);
        assert_eq!(client.blob.get(&sha, 0, None).await.unwrap(), FIG_1);
    }

    // ── image failures and picture formats (US-0006) ────────────────────────

    /// `n` PNG pictures, captioned `cap1`..`capN`, one reference each, page
    /// `i` for the i-th, every target and file present (FR-NEW-015).
    fn captioned_pictures(n: usize) -> Conversion {
        let targets: Vec<String> = (1..=n).map(|i| format!("p/{i}.png")).collect();
        let bytes: Vec<Vec<u8>> = (1..=n).map(|i| format!("png {i}").into_bytes()).collect();
        let captions: Vec<String> = (1..=n).map(|i| format!("cap{i}")).collect();
        let md: String = targets.iter().map(|t| format!("![]({t})\n\n")).collect();
        let pics: Vec<Pic<'_>> = targets
            .iter()
            .zip(&bytes)
            .zip(&captions)
            .zip(1i64..)
            .map(|(((t, b), c), page)| Pic { target: t, bytes: b, caption: c, page: Some(page) })
            .collect();
        bundle(&md, &pics)
    }

    /// `conversion`, its manifest entry for `target` marked caption failed.
    fn with_caption_failed(mut conversion: Conversion, target: &str) -> Conversion {
        if let Some(meta) = conversion.manifest.as_mut().and_then(|m| m.pictures.get_mut(target)) {
            meta.caption_failed = true;
        }
        conversion
    }

    /// `n` pictures referenced in the text, the `missing`-th (1 based) absent
    /// from the converter's files, every other present.
    fn conversion_with_missing(n: usize, missing: usize) -> Conversion {
        let targets: Vec<String> = (1..=n).map(|i| format!("p/{i}.png")).collect();
        let md: String = targets.iter().map(|t| format!("![]({t})\n\n")).collect();
        let mut files = std::collections::BTreeMap::new();
        let mut pictures = std::collections::BTreeMap::new();
        for (idx, t) in (1..=n).zip(&targets) {
            if idx == missing {
                continue;
            }
            files.insert(t.clone(), format!("png {idx}").into_bytes());
            pictures.insert(
                t.clone(),
                PictureMeta {
                    caption: String::new(),
                    caption_failed: false,
                    page: Some(idx as i64),
                },
            );
        }
        Conversion { markdown: md, files, manifest: Some(Manifest { pictures }) }
    }

    /// SPEC-0019/E2E-005: a failed caption is reported and the image is kept
    /// with an empty caption, every other image unaffected.
    #[tokio::test]
    async fn spec_0019_e2e_005_a_failed_caption_is_reported_and_the_image_kept() {
        let conversion = with_caption_failed(captioned_pictures(5), "p/4.png");
        let (_f, server) = converting_bundle("/deck.pdf", conversion).await;
        let out = convert(&server, "/deck.pdf").await;
        assert_eq!(out["artifacts_report"], "5 images, 0 tables\nimage-4: caption unavailable");
        let list = images(&server, "/deck.pdf", None).await;
        let captions: Vec<Value> =
            list["images"].as_array().unwrap().iter().map(|i| i["caption"].clone()).collect();
        assert_eq!(
            captions,
            vec![json!("cap1"), json!("cap2"), json!("cap3"), json!(""), json!("cap5")]
        );
    }

    /// SPEC-0019/E2E-007: a referenced picture missing from the converter's
    /// files is reported and its id stays unused.
    #[tokio::test]
    async fn spec_0019_e2e_007_a_missing_referenced_picture_is_reported() {
        let conversion = conversion_with_missing(5, 3);
        let (_f, server) = converting_bundle("/deck2.pdf", conversion).await;
        let out = convert(&server, "/deck2.pdf").await;
        assert_eq!(out["artifacts_report"], "4 images, 0 tables\nimage-3: picture unavailable");
        let list = images(&server, "/deck2.pdf", None).await;
        assert_eq!(image_ids(&list), vec!["image-1", "image-2", "image-4", "image-5"]);
    }

    /// SPEC-0019/E2E-008: an unsupported picture format is reported and not
    /// kept; a supported one beside it still comes through.
    #[tokio::test]
    async fn spec_0019_e2e_008_an_unsupported_picture_format_is_reported() {
        let md = "![](a.jpg)\n\n![](b.svg)\n";
        let files: std::collections::BTreeMap<String, Vec<u8>> = [
            ("a.jpg".to_string(), b"jpeg bytes".to_vec()),
            ("b.svg".to_string(), b"<svg/>".to_vec()),
        ]
        .into();
        let conversion = Conversion { markdown: md.to_string(), files, manifest: None };
        let (_f, server) = converting_bundle("/mixed.pdf", conversion).await;
        let out = convert(&server, "/mixed.pdf").await;
        assert_eq!(
            out["artifacts_report"],
            "1 images, 0 tables\nimage-2: unsupported picture format svg"
        );
        let got = image(&server, "/mixed.pdf", "image-1").await;
        assert_eq!(got["format"], "jpeg");
    }

    /// SPEC-0019/E2E-113: a `.tif` picture is kept as format `tiff`, with no
    /// missing item.
    #[tokio::test]
    async fn spec_0019_e2e_113_a_tif_picture_is_kept_as_tiff() {
        let md = "![](scan.tif)\n";
        let files: std::collections::BTreeMap<String, Vec<u8>> =
            [("scan.tif".to_string(), b"tiff bytes".to_vec())].into();
        let conversion = Conversion { markdown: md.to_string(), files, manifest: None };
        let (_f, server) = converting_bundle("/scan2.pdf", conversion).await;
        let out = convert(&server, "/scan2.pdf").await;
        assert_eq!(out["artifacts_report"], "1 images, 0 tables");
        let got = image(&server, "/scan2.pdf", "image-1").await;
        assert_eq!(got["format"], "tiff");
    }

    /// SPEC-0019/E2E-114: an image failure then a table failure are reported
    /// in that order, images before tables.
    #[tokio::test]
    async fn spec_0019_e2e_114_image_and_table_failures_are_reported_in_order() {
        let md = "![](p/1.png)\n\n![](p/2.png)\n\n![](p/3.png)\n\n\
                  | a | b |\n| --- |\n\n\
                  | c |\n| --- |\n";
        let mut files = std::collections::BTreeMap::new();
        let mut pictures = std::collections::BTreeMap::new();
        for i in [1, 3] {
            let t = format!("p/{i}.png");
            files.insert(t.clone(), format!("png {i}").into_bytes());
            pictures.insert(
                t,
                PictureMeta { caption: String::new(), caption_failed: false, page: Some(i) },
            );
        }
        let conversion =
            Conversion { markdown: md.to_string(), files, manifest: Some(Manifest { pictures }) };
        let (_f, server) = converting_bundle("/mix2.pdf", conversion).await;
        let out = convert(&server, "/mix2.pdf").await;
        assert_eq!(
            out["artifacts_report"],
            "2 images, 1 tables\nimage-2: picture unavailable\ntable-1: malformed table"
        );
    }

    /// SPEC-0019/E2E-129: a single failed caption is reported, the image kept
    /// with an empty caption.
    #[tokio::test]
    async fn spec_0019_e2e_129_a_single_failed_caption_is_reported() {
        let conversion = with_caption_failed(captioned_pictures(1), "p/1.png");
        let (_f, server) = converting_bundle("/blur.pdf", conversion).await;
        let out = convert(&server, "/blur.pdf").await;
        assert_eq!(out["artifacts_report"], "1 images, 0 tables\nimage-1: caption unavailable");
        assert_eq!(images(&server, "/blur.pdf", None).await["images"][0]["caption"], "");
    }

    /// SPEC-0019/E2E-143: a picture the converter drops entirely (not
    /// referenced in the text) takes no id and is not reported.
    #[tokio::test]
    async fn spec_0019_e2e_143_a_converter_dropped_picture_is_not_reported() {
        let md = "![](p/1.png)\n\n![](p/2.png)\n";
        let files: std::collections::BTreeMap<String, Vec<u8>> =
            [("p/1.png".to_string(), b"one".to_vec()), ("p/2.png".to_string(), b"two".to_vec())]
                .into();
        let conversion = Conversion { markdown: md.to_string(), files, manifest: None };
        let (_f, server) = converting_bundle("/drop.pdf", conversion).await;
        let out = convert(&server, "/drop.pdf").await;
        assert_eq!(out["artifacts_report"], "2 images, 0 tables");
        let list = images(&server, "/drop.pdf", None).await;
        assert_eq!(image_ids(&list), vec!["image-1", "image-2"]);
    }

    /// A shell script `document.md` producer run through the real
    /// [`CliDocService`], `{outdir}` mode: writes `document.md`, a kept
    /// picture, and (SPEC-0019/E2E-151) a `secret.png` at the exact
    /// filesystem location its own `../../secret.png` markdown reference
    /// would resolve to, so the only thing standing between the conversion
    /// and that file is the real escape rejection, not an in-memory stub.
    fn write_traversal_bundle_script(dir: &std::path::Path, secret: &[u8]) -> std::path::PathBuf {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("bundle.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        // $1 is the outdir ({outdir}), $2 is the staged input ({document}).
        writeln!(f, "out=\"$1\"").unwrap();
        writeln!(f, "mkdir -p \"$out/figures\"").unwrap();
        writeln!(
            f,
            "printf '![x](../../secret.png)\\n\\n![y](figures/figure_1.png)\\n' > \"$out/document.md\""
        )
        .unwrap();
        writeln!(f, "printf 'fig1' > \"$out/figures/figure_1.png\"").unwrap();
        // Resolve where `../../secret.png` really lands from $out, by
        // navigating there, not by assuming a path: this is the exact
        // location the production target resolution would read from if it
        // ever followed `..` onto disk.
        writeln!(f, "dest=$(cd \"$out/../..\" && pwd)").unwrap();
        writeln!(f, "printf '%s' '{}' > \"$dest/secret.png\"", String::from_utf8_lossy(secret))
            .unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// SPEC-0019/E2E-151: a target escaping the converter's output with `..`
    /// is reported and never read, even when a real file sits there, driven
    /// through the real CLI document service (not an in-memory stub), so a
    /// future regression that let a target resolve onto disk would actually
    /// be caught.
    #[tokio::test]
    async fn spec_0019_e2e_151_a_path_escaping_the_output_is_never_read() {
        let secret = b"top secret pixels";
        let home = tempfile::tempdir().expect("a temp dir for the fake converter script");
        let sh = write_traversal_bundle_script(home.path(), secret);

        let cli = crate::docs::CliDocService::new(
            crate::config::DocServiceCliConfig {
                command: vec![
                    sh.display().to_string(),
                    crate::config::OUTDIR_PLACEHOLDER.into(),
                    crate::config::DOC_PLACEHOLDER.into(),
                ],
                timeout_secs: 30,
            },
            crate::docs::DOC_SERVICE_EXTS.iter().map(|e| (*e).to_string()).collect(),
            u64::MAX,
        );

        let (f, _) = setup(None).await;
        put(&f, "/trav.pdf", b"%PDF-1.4 source").await;
        let server = with_doc(&f, Some(Arc::new(cli)));

        let out = convert(&server, "/trav.pdf").await;
        assert_eq!(out["artifacts_report"], "1 images, 0 tables\nimage-1: picture unavailable");
        let list = images(&server, "/trav.pdf", None).await;
        assert_eq!(image_ids(&list), vec!["image-2"]);

        let client = f.state.stores.client(PROJECT).await.unwrap();
        let sha = crate::storage::VolumeClient::sha256_hex(secret);
        assert!(client.blob.get(&sha, 0, None).await.is_err(), "no blob for the secret's bytes");

        // Cleanup: the planted file sits two levels above the sandbox, which
        // is outside the tempdir that CliDocService itself removes.
        let _ = std::fs::remove_file(std::env::temp_dir().join("secret.png"));
    }

    /// SPEC-0019/E2E-152: a URL target is reported and never fetched.
    #[tokio::test]
    async fn spec_0019_e2e_152_a_url_target_is_never_read() {
        let md = "![x](https://example.com/a.png)\n";
        let conversion = Conversion {
            markdown: md.to_string(),
            files: std::collections::BTreeMap::new(),
            manifest: None,
        };
        let (_f, server) = converting_bundle("/url.pdf", conversion).await;
        let out = convert(&server, "/url.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 0 tables\nimage-1: picture unavailable");
    }

    /// SPEC-0019/E2E-153: an absolute path target is reported and never read
    /// from disk.
    #[tokio::test]
    async fn spec_0019_e2e_153_an_absolute_target_is_never_read() {
        let md = "![x](/etc/hosts.png)\n";
        let conversion = Conversion {
            markdown: md.to_string(),
            files: std::collections::BTreeMap::new(),
            manifest: None,
        };
        let (_f, server) = converting_bundle("/abs.pdf", conversion).await;
        let out = convert(&server, "/abs.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 0 tables\nimage-1: picture unavailable");
    }

    // ── images through every conversion path (US-0007) ──────────────────────

    /// A few PNG signature bytes: an image the built-in conversion reads no
    /// text from, the OCR provider being null by default.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    async fn no_artifacts(server: &McpServer, path: &str) {
        let err = err_text(server.fs_list_images(Parameters(list_images_args(path, None))).await);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    fn empty_images() -> Value {
        json!({"images": [], "marker": null})
    }

    fn qualities(list: &Value) -> Vec<String> {
        list["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["csv_quality"].as_str().unwrap().to_string())
            .collect()
    }

    fn pdf_only(markdown: &str) -> Arc<dyn DocService> {
        Arc::new(StubDocService::ok(markdown).with_extensions(&[".pdf"]))
    }

    /// SPEC-0019/E2E-002: no converter, the built-in conversion keeps both
    /// Excel tables, exact, and no image (FR-NEW-017).
    #[tokio::test]
    async fn spec_0019_e2e_002_builtin_excel_keeps_exact_tables_and_no_image() {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, None);
        put(&f, "/budget.xlsx", &two_xlsx()).await;
        let out = ok_json(
            server.fs_extract_text(Parameters(extract_args("/budget.xlsx", 200_000))).await,
        );
        assert_eq!(out["artifacts_report"], "0 images, 2 tables");
        let list = tables(&server, "/budget.xlsx").await;
        assert_eq!(qualities(&list), vec!["CSV quality: exact", "CSV quality: exact"]);
        assert_eq!(images(&server, "/budget.xlsx", None).await, empty_images());
    }

    /// SPEC-0019/E2E-003: a converter configured for PDF only does not change
    /// what text extraction keeps: the built-in tables, exact, no image.
    #[tokio::test]
    async fn spec_0019_e2e_003_extraction_ignores_the_converter() {
        let (f, _) = setup(None).await;
        // Were the converter used, it would yield one approximate table.
        let server = with_doc(&f, Some(pdf_only("| z |\n| --- |\n| 9 |\n")));
        put(&f, "/budget.xlsx", &two_xlsx()).await;
        ok_json(server.fs_extract_text(Parameters(extract_args("/budget.xlsx", 200_000))).await);
        let list = tables(&server, "/budget.xlsx").await;
        assert_eq!(ids(&list), vec!["table-1", "table-2"]);
        assert_eq!(qualities(&list), vec!["CSV quality: exact", "CSV quality: exact"]);
        assert_eq!(images(&server, "/budget.xlsx", None).await, empty_images());
    }

    /// SPEC-0019/E2E-004: a conversion over the session quota is refused and
    /// keeps no set and no sibling (FR-NEW-013).
    #[tokio::test]
    async fn spec_0019_e2e_004_a_conversion_over_the_quota_keeps_nothing() {
        // Sibling 25 000 and conversion text 25 000: 50 000 together.
        let (f, server) = converting("/report.pdf", &"z".repeat(25_000), Some(QUOTA)).await;
        f.state.safety.charge_write(ALICE, PROJECT, 250_000).unwrap();
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/report.pdf", false))).await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        no_artifacts(&server, "/report.pdf").await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/report.md").await.unwrap(), "no sibling");
    }

    /// SPEC-0019/E2E-009: no converter, a PDF goes through the built-in
    /// conversion, which keeps no image and no PDF table (FR-NEW-017).
    #[tokio::test]
    async fn spec_0019_e2e_009_builtin_pdf_keeps_no_image_and_no_table() {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, None);
        put(&f, "/scan.pdf", &crate::docs::extract::tests::tiny_pdf()).await;
        let out =
            ok_json(server.fs_extract_text(Parameters(extract_args("/scan.pdf", 200_000))).await);
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");
        assert_eq!(images(&server, "/scan.pdf", None).await, empty_images());
        assert_eq!(tables(&server, "/scan.pdf").await, json!({"tables": [], "marker": null}));
    }

    /// SPEC-0019/E2E-012 then E2E-017: a documented upload over the quota
    /// keeps the source, charged alone, reports the quota message as its
    /// documentation outcome, and keeps no sibling and no set (FR-NEW-014).
    #[tokio::test]
    async fn spec_0019_e2e_012_017_a_documented_upload_over_the_quota_charges_the_source_only() {
        let (f, _) = setup(Some(QUOTA)).await;
        // Sibling 50 000 and conversion text 50 000: 100 000 together.
        let server = with_doc(&f, Some(Arc::new(StubDocService::ok("z".repeat(50_000)))));
        let source = vec![b'p'; 200_000];

        // E2E-012.
        let out = ok_json(
            server.fs_write_bytes(Parameters(write_args("/report.pdf", &source, true))).await,
        );
        assert_eq!(out["path"], "/report.pdf");
        assert_eq!(out["bytes_written"], 200_000);
        assert_eq!(
            out["documentation"]["error"]["message"],
            "session write quota of 262144 bytes exceeded",
            "got: {out}"
        );
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert_eq!(client.read_bytes("/report.pdf").await.unwrap(), source);
        assert!(!client.exists("/report.md").await.unwrap(), "no sibling");
        no_artifacts(&server, "/report.pdf").await;

        // E2E-017: only the 200 000 source bytes were charged.
        ok_json(
            server
                .fs_write_bytes(Parameters(write_args("/notes.txt", &[b'n'; 62_144], false)))
                .await,
        );
        let err =
            err_text(server.fs_write_bytes(Parameters(write_args("/one.txt", b"x", false))).await);
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
    }

    /// SPEC-0019/E2E-013: a non member converting gets the usual refusal and
    /// leaves no set behind (FR-NEW-016).
    #[tokio::test]
    async fn spec_0019_e2e_013_a_non_member_cannot_convert() {
        let (f, server) = converting_bundle("/report.pdf", report_bundle()).await;
        let doc: Arc<dyn DocService> = Arc::new(StubDocService::bundle(report_bundle()));
        let olga = with_doc_as(&f, Some(doc), "olga@test.com");
        let usual =
            err_text(olga.fs_extract_text(Parameters(extract_args("/report.pdf", 200_000))).await);
        assert!(usual.contains("ERR_FORBIDDEN"), "got: {usual}");
        let refusal = |e: &str| e.split_once("': ").map(|(_, r)| r.to_string()).unwrap();
        let got =
            err_text(olga.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await);
        assert_eq!(refusal(&got), refusal(&usual));
        no_artifacts(&server, "/report.pdf").await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/report.md").await.unwrap(), "no sibling");
    }

    /// SPEC-0019/E2E-015: a text only document reports and lists nothing.
    #[tokio::test]
    async fn spec_0019_e2e_015_a_text_only_document_keeps_nothing() {
        let (_f, server) = converting("/plain.pdf", "# Plain\n\nJust prose.\n", None).await;
        assert_eq!(convert(&server, "/plain.pdf").await["artifacts_report"], "0 images, 0 tables");
        assert_eq!(images(&server, "/plain.pdf", None).await, empty_images());
        assert_eq!(tables(&server, "/plain.pdf").await, json!({"tables": [], "marker": null}));
    }

    /// SPEC-0019/E2E-016: an image extracted while the converter takes PDF
    /// only goes through the built-in conversion: no image kept.
    #[tokio::test]
    async fn spec_0019_e2e_016_an_extracted_image_keeps_no_image() {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, Some(pdf_only("x")));
        put(&f, "/photo.png", PNG).await;
        let out =
            ok_json(server.fs_extract_text(Parameters(extract_args("/photo.png", 200_000))).await);
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");
        assert_eq!(images(&server, "/photo.png", None).await, empty_images());
    }

    /// SPEC-0019/E2E-018: a never converted document with no sibling converts
    /// without overwrite and keeps its image (FR-NEW-027).
    #[tokio::test]
    async fn spec_0019_e2e_018_a_first_conversion_needs_no_overwrite() {
        let conversion = bundle(
            "![](f/1.png)\n",
            &[Pic { target: "f/1.png", bytes: b"fresh", caption: "", page: Some(1) }],
        );
        let (_f, server) = converting_bundle("/fresh.pdf", conversion).await;
        let out = convert(&server, "/fresh.pdf").await;
        assert_eq!(out["artifacts_report"], "1 images, 0 tables");
        assert_eq!(out["overwritten"], false);
        assert_eq!(image_ids(&images(&server, "/fresh.pdf", None).await), vec!["image-1"]);
    }

    /// SPEC-0019/E2E-100: an image yielding no text writes no sibling, yet
    /// counts as converted: an empty report and an empty list.
    #[tokio::test]
    async fn spec_0019_e2e_100_a_textless_image_is_converted_empty() {
        let (f, _) = setup(None).await;
        let server = with_doc(&f, None);
        put(&f, "/blank.png", PNG).await;
        let out =
            ok_json(server.fs_extract_text(Parameters(extract_args("/blank.png", 200_000))).await);
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/blank.md").await.unwrap(), "no empty sibling");
        assert_eq!(images(&server, "/blank.png", None).await, empty_images());
    }

    /// SPEC-0019/E2E-111: the convert action on a format the converter
    /// refuses answers today's refusal verbatim and keeps no set (FR-NEW-018).
    #[tokio::test]
    async fn spec_0019_e2e_111_a_refused_format_is_refused_verbatim() {
        let (f, _) = setup(None).await;
        put(&f, "/photo.png", PNG).await;
        let server = with_doc(&f, Some(pdf_only("x")));
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/photo.png", false))).await,
        );
        assert!(
            err.contains("'/photo.png' cannot be documented, the document service accepts: .pdf"),
            "got: {err}"
        );
        no_artifacts(&server, "/photo.png").await;
    }

    /// SPEC-0019/E2E-124: the convert action keeps every key of today and
    /// adds the report (FR-NEW-030, FR-MOD-002).
    #[tokio::test]
    async fn spec_0019_e2e_124_convert_keeps_its_keys_and_reports() {
        let (_f, _server, out) = report_with_images().await;
        let mut keys: Vec<&String> = out.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["artifacts_report", "bytes_written", "md_path", "overwritten", "path"],
            "got: {out}"
        );
        assert_eq!(out["path"], "/report.pdf");
        assert_eq!(out["md_path"], "/report.md");
        assert_eq!(out["artifacts_report"], "2 images, 2 tables");
    }

    /// SPEC-0019/E2E-125: a documented upload carries the sibling path as
    /// today and the report (FR-NEW-030, FR-MOD-002).
    #[tokio::test]
    async fn spec_0019_e2e_125_a_documented_upload_reports() {
        let (f, _) = setup(None).await;
        let server = bundling(&f, report_bundle());
        let out = ok_json(
            server.fs_write_bytes(Parameters(write_args("/report.pdf", b"%PDF-1.4", true))).await,
        );
        let doc = &out["documentation"];
        assert_eq!(doc["md_path"], "/report.md");
        assert_eq!(doc["artifacts_report"], "2 images, 2 tables");
        assert_eq!(
            image_ids(&images(&server, "/report.pdf", None).await),
            vec!["image-1", "image-2"]
        );
    }

    // ── full document view (US-0008) ────────────────────────────────────────

    /// The Markdown rendering of `report.pdf`'s first table (E2E-035).
    const Q1_MD: [&str; 5] = [
        "| Region | Jan | Feb | Mar |",
        "| --- | --- | --- | --- |",
        "| North | 10 | 11 | 12 |",
        "| South | 20 | 21 | 22 |",
        "| West | 30 | 31 | 32 |",
    ];
    /// The Markdown rendering of `report.pdf`'s second table (E2E-048).
    const COSTS_MD: [&str; 4] =
        ["| Item | EUR |", "| --- | --- |", "| Rent | 900 |", "| Power | 120 |"];

    fn view_args(path: &str, captions: bool, mode: Option<&str>) -> DocumentViewArgs {
        DocumentViewArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            captions,
            table_mode: mode.map(str::to_string),
        }
    }

    async fn view(server: &McpServer, path: &str, captions: bool, mode: Option<&str>) -> String {
        let out =
            ok_json(server.fs_get_document_view(Parameters(view_args(path, captions, mode))).await);
        assert_eq!(out["path"], path, "got: {out}");
        out["text"].as_str().expect("text").to_string()
    }

    async fn view_err(server: &McpServer, path: &str, mode: Option<&str>) -> String {
        err_text(server.fs_get_document_view(Parameters(view_args(path, true, mode))).await)
    }

    /// The index of the only line equal to `want`, failing on none or several.
    fn only_line(lines: &[&str], want: &str) -> usize {
        let at: Vec<usize> = (0..lines.len()).filter(|i| lines[*i] == want).collect();
        assert_eq!(at.len(), 1, "{want:?} must stand exactly once in {lines:?}");
        at[0]
    }

    /// The non empty lines following line `at`, as many as `want` holds.
    fn next_non_empty<'a>(lines: &[&'a str], at: usize, n: usize) -> Vec<&'a str> {
        lines[at + 1..].iter().copied().filter(|l| !l.trim().is_empty()).take(n).collect()
    }

    /// An image line stands between empty lines, or at an end of the view.
    fn between_empty_lines(lines: &[&str], at: usize) {
        assert!(at == 0 || lines[at - 1].trim().is_empty(), "before {:?}: {lines:?}", lines[at]);
        assert!(
            at + 1 == lines.len() || lines[at + 1].trim().is_empty(),
            "after {:?}: {lines:?}",
            lines[at]
        );
    }

    /// SPEC-0019/E2E-050: by default the first image line stands at the first
    /// image's place and both tables render as Markdown at theirs (FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_050_default_view_places_images_and_markdown_tables() {
        let (_f, server, _) = report_with_images().await;
        let text = view(&server, "/report.pdf", true, None).await;
        let lines: Vec<&str> = text.lines().collect();
        let first = lines.iter().position(|l| !l.trim().is_empty()).expect("a non empty line");
        assert_eq!(lines[first], "[Image image-1: Figure 1: Revenue 2024]", "got: {text}");
        let q1 = only_line(&lines, "Table 1: Q1 sales");
        assert_eq!(next_non_empty(&lines, q1, 5), Q1_MD, "got: {text}");
        let costs = only_line(&lines, "Table 2: Costs");
        assert_eq!(next_non_empty(&lines, costs, 4), COSTS_MD, "got: {text}");
        // The block is rendered whole, contiguous, as FR-NEW-002 writes it.
        assert!(text.contains(&format!("{}\n", Q1_MD.join("\n"))), "got: {text}");
        assert!(text.contains(&format!("{}\n", COSTS_MD.join("\n"))), "got: {text}");
        assert!(!text.contains("figures/f1.png") && !text.contains("figures/f2.png"));
    }

    /// SPEC-0019/E2E-051: `csv-reference` renders each table as its reference
    /// line and no Markdown table (FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_051_csv_reference_replaces_tables_by_reference_lines() {
        let (_f, server, _) = report_with_images().await;
        let text = view(&server, "/report.pdf", true, Some("csv-reference")).await;
        let lines: Vec<&str> = text.lines().collect();
        let q1 = only_line(&lines, "Table 1: Q1 sales");
        assert_eq!(next_non_empty(&lines, q1, 1), ["[Table table-1: Q1 sales (CSV)]"]);
        let costs = only_line(&lines, "Table 2: Costs");
        assert_eq!(next_non_empty(&lines, costs, 1), ["[Table table-2: Costs (CSV)]"]);
        assert!(lines.iter().all(|l| !l.trim_start().starts_with('|')), "got: {text}");
    }

    /// SPEC-0019/E2E-052: `both` with captions off: each Markdown table then
    /// its reference line, and image lines without captions (FR-NEW-007, 008).
    #[tokio::test]
    async fn spec_0019_e2e_052_both_mode_with_captions_off() {
        let (_f, server, _) = report_with_images().await;
        let text = view(&server, "/report.pdf", false, Some("both")).await;
        let mut q1 = Q1_MD.join("\n");
        q1.push_str("\n[Table table-1: Q1 sales (CSV)]\n");
        let mut costs = COSTS_MD.join("\n");
        costs.push_str("\n[Table table-2: Costs (CSV)]\n");
        assert!(text.contains(&q1), "got: {text}");
        assert!(text.contains(&costs), "got: {text}");
        let lines: Vec<&str> = text.lines().collect();
        let one = only_line(&lines, "[Image image-1]");
        let two = only_line(&lines, "[Image image-2]");
        assert!(one < two);
    }

    /// SPEC-0019/E2E-053: captions off in `markdown` mode hides every caption
    /// and keeps both Markdown tables (FR-NEW-008).
    #[tokio::test]
    async fn spec_0019_e2e_053_captions_off_hides_image_captions() {
        let (_f, server, _) = report_with_images().await;
        let text = view(&server, "/report.pdf", false, Some("markdown")).await;
        let lines: Vec<&str> = text.lines().collect();
        only_line(&lines, "[Image image-1]");
        only_line(&lines, "[Image image-2]");
        assert!(!text.contains("Figure 1: Revenue 2024"), "got: {text}");
        assert!(!text.contains("Schéma réseau 🌐"), "got: {text}");
        assert!(text.contains(&Q1_MD.join("\n")) && text.contains(&COSTS_MD.join("\n")));
    }

    async fn unsupported_mode(mode: &str) {
        let (_f, server, _) = report_with_images().await;
        let err = view_err(&server, "/report.pdf", Some(mode)).await;
        assert!(err.contains("ERR_INVALID_ARGUMENT"), "got: {err}");
        assert!(
            err.contains(&format!(
                "Unsupported table mode: {mode} (use markdown, csv-reference or both)"
            )),
            "got: {err}"
        );
        assert!(!err.contains("Report") && !err.contains("[Image"), "no text: {err}");
    }

    /// SPEC-0019/E2E-054: `html` is no table mode (FR-NEW-025).
    #[tokio::test]
    async fn spec_0019_e2e_054_html_is_refused() {
        unsupported_mode("html").await;
    }

    /// SPEC-0019/E2E-055: modes are case sensitive (FR-NEW-025).
    #[tokio::test]
    async fn spec_0019_e2e_055_upper_case_mode_is_refused() {
        unsupported_mode("CSV-REFERENCE").await;
    }

    /// SPEC-0019/E2E-056: a list of modes is no mode (FR-NEW-025).
    #[tokio::test]
    async fn spec_0019_e2e_056_a_list_of_modes_is_refused() {
        unsupported_mode("markdown,both").await;
    }

    /// SPEC-0019/E2E-057: a never converted document has no view, and asking
    /// creates no sibling (FR-NEW-023).
    #[tokio::test]
    async fn spec_0019_e2e_057_a_never_converted_document_has_no_view() {
        let (f, server) = converting_bundle("/notes.pdf", report_bundle()).await;
        let err = view_err(&server, "/notes.pdf", None).await;
        assert!(
            err.contains("ERR_NOT_FOUND") && err.contains("No artifacts: document not extracted"),
            "got: {err}"
        );
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/notes.md").await.unwrap(), "no sibling created");
    }

    /// SPEC-0019/E2E-058: a non member gets the usual refusal, no text
    /// (FR-NEW-016).
    #[tokio::test]
    async fn spec_0019_e2e_058_a_non_member_gets_the_usual_refusal() {
        let (f, _, _) = report_with_images().await;
        let olga = McpServer::new(f.state.clone(), "olga@test.com".to_string());
        let usual =
            err_text(olga.fs_extract_text(Parameters(extract_args("/report.pdf", 200_000))).await);
        assert!(usual.contains("ERR_FORBIDDEN"), "got: {usual}");
        let err = err_text(
            olga.fs_get_document_view(Parameters(view_args("/report.pdf", true, Some("both"))))
                .await,
        );
        let refusal = |e: &str| e.split_once("': ").map(|(_, r)| r.to_string()).unwrap();
        assert_eq!(refusal(&err), refusal(&usual));
        assert!(!err.contains("Report") && !err.contains("[Image"), "got: {err}");
    }

    /// SPEC-0019/E2E-059: after the sibling is deleted and the source
    /// overwritten, the document is not extracted (FR-NEW-009, FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_059_an_overwritten_source_has_no_view() {
        let (f, server, _) = report_with_images().await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        client.delete_file("/report.md").await.unwrap();
        put(&f, "/report.pdf", b"%PDF-1.4 rewritten").await;
        let err = view_err(&server, "/report.pdf", None).await;
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// SPEC-0019/E2E-060: a table without caption has a reference line
    /// without caption (FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_060_an_uncaptioned_table_reference() {
        let md = format!("{REPORT_MD}\nNotes\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n");
        let (_f, server) = converting("/report3.pdf", &md, None).await;
        convert(&server, "/report3.pdf").await;
        let text = view(&server, "/report3.pdf", true, Some("csv-reference")).await;
        let lines: Vec<&str> = text.lines().collect();
        let first = only_line(&lines, "[Table table-1: Q1 sales (CSV)]");
        let second = only_line(&lines, "[Table table-2: Costs (CSV)]");
        let third = only_line(&lines, "[Table table-3 (CSV)]");
        assert!(first < second && second < third, "got: {text}");
    }

    /// SPEC-0019/E2E-061: with no image, captions on and off give the same
    /// bytes (FR-NEW-008).
    #[tokio::test]
    async fn spec_0019_e2e_061_no_image_captions_change_nothing() {
        let md =
            "# Plain\n\nTable 1: Q1 sales\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nJust prose.\n";
        let (_f, server) = converting("/plain.pdf", md, None).await;
        convert(&server, "/plain.pdf").await;
        for mode in [None, Some("csv-reference"), Some("both")] {
            let off = view(&server, "/plain.pdf", false, mode).await;
            let on = view(&server, "/plain.pdf", true, mode).await;
            assert_eq!(off, on, "{mode:?}");
        }
        // Table reference lines keep their caption with captions off.
        let off = view(&server, "/plain.pdf", false, Some("csv-reference")).await;
        assert!(off.contains("\n[Table table-1: Q1 sales (CSV)]\n"), "got: {off}");
    }

    /// SPEC-0019/E2E-062: an image without caption has an image line without
    /// caption, even with captions on (FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_062_an_uncaptioned_image_line() {
        let conversion = bundle(
            "Intro\n\n![](img/a.png)\n",
            &[Pic { target: "img/a.png", bytes: b"one", caption: "", page: Some(2) }],
        );
        let (_f, server) = converting_bundle("/one.pdf", conversion).await;
        convert(&server, "/one.pdf").await;
        let text = view(&server, "/one.pdf", true, None).await;
        let lines: Vec<&str> = text.lines().collect();
        only_line(&lines, "[Image image-1]");
        assert!(text.starts_with("Intro\n"), "got: {text}");
        assert!(!text.contains("img/a.png"), "got: {text}");
    }

    /// SPEC-0019/E2E-063: each image line stands once, in reading order,
    /// between empty lines (FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_063_image_lines_stand_once_between_empty_lines() {
        let (_f, server, _) = report_with_images().await;
        let text = view(&server, "/report.pdf", true, None).await;
        let lines: Vec<&str> = text.lines().collect();
        let one = only_line(&lines, "[Image image-1: Figure 1: Revenue 2024]");
        let two = only_line(&lines, "[Image image-2: Schéma réseau 🌐]");
        assert!(one < two, "got: {text}");
        between_empty_lines(&lines, one);
        between_empty_lines(&lines, two);
    }

    /// SPEC-0019/E2E-068: views never change the sibling (FR-NEW-022).
    #[tokio::test]
    async fn spec_0019_e2e_068_views_leave_the_sibling_unchanged() {
        let (f, server, _) = report_with_images().await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let before = client.read_text("/report.md").await.unwrap();
        for mode in ["markdown", "csv-reference", "both"] {
            view(&server, "/report.pdf", true, Some(mode)).await;
            view(&server, "/report.pdf", false, Some(mode)).await;
        }
        assert_eq!(client.read_text("/report.md").await.unwrap(), before);
    }

    /// SPEC-0019/E2E-116: a text extraction with refresh replaces the set with
    /// an empty built-in one, and the default view is then the sibling byte for
    /// byte (FR-NEW-030, FR-MOD-002).
    #[tokio::test]
    async fn spec_0019_e2e_116_a_refreshed_extraction_views_as_its_sibling() {
        let (f, _) = setup(None).await;
        put(&f, "/report.pdf", &crate::docs::extract::tests::tiny_pdf()).await;
        let server = bundling(&f, report_bundle());
        convert(&server, "/report.pdf").await;
        let mut args = extract_args("/report.pdf", 200_000);
        args.refresh = true;
        let out = ok_json(server.fs_extract_text(Parameters(args)).await);
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let sibling = client.read_text("/report.md").await.unwrap();
        assert!(!sibling.is_empty());
        assert_eq!(view(&server, "/report.pdf", true, Some("markdown")).await, sibling);
    }

    /// SPEC-0019/E2E-119: a missing document is `not a file`, before the mode
    /// is checked (FR-NEW-031).
    #[tokio::test]
    async fn spec_0019_e2e_119_not_a_file_before_the_mode() {
        let (_f, server) = setup(None).await;
        let err = view_err(&server, "/ghost.pdf", Some("html")).await;
        assert!(err.contains("ERR_NOT_FOUND") && err.contains("not a file: /ghost.pdf"), "{err}");
        assert!(!err.contains("Unsupported table mode"), "{err}");
    }

    /// SPEC-0019/E2E-132: a cut extraction views up to its cut, its one kept
    /// table once (FR-NEW-034).
    #[tokio::test]
    async fn spec_0019_e2e_132_a_cut_extraction_views_to_its_cut() {
        let (f, server) = setup(None).await;
        put(&f, "/two.xlsx", &two_xlsx()).await;
        let full =
            ok_json(server.fs_extract_text(Parameters(extract_args("/two.xlsx", 200_000))).await);
        let text = full["preview"].as_str().unwrap().to_string();
        let at = text.find("| bx0").expect("B's first data row");
        let cut = text[..at].chars().count() as i64;
        let mut args = extract_args("/two.xlsx", cut);
        args.refresh = true;
        ok_json(server.fs_extract_text(Parameters(args)).await);

        let viewed = view(&server, "/two.xlsx", true, None).await;
        assert_eq!(viewed.trim_end(), text[..at].trim_end(), "the view ends at the cut");
        let refs = view(&server, "/two.xlsx", true, Some("csv-reference")).await;
        assert_eq!(refs.matches("table-1").count(), 1, "got: {refs}");
        assert!(!refs.contains("table-2"), "got: {refs}");
        assert_eq!(refs.matches("| ax0").count(), 0, "table A is its reference: {refs}");
    }

    /// FR-NEW-007: a caption repeated on the first non empty line after the
    /// picture reference is removed with it; nothing else is.
    #[tokio::test]
    async fn a_caption_line_after_the_picture_is_removed_with_it() {
        let conversion = bundle(
            "Intro\n![](a.png)\n\nFigure 1: Chart\n\nFigure 1: Chart\nOutro\n",
            &[Pic { target: "a.png", bytes: b"a", caption: "Figure 1: Chart", page: None }],
        );
        let (_f, server) = converting_bundle("/c.pdf", conversion).await;
        convert(&server, "/c.pdf").await;
        let text = view(&server, "/c.pdf", false, None).await;
        let lines: Vec<&str> = text.lines().collect();
        only_line(&lines, "[Image image-1]");
        only_line(&lines, "Figure 1: Chart");
        assert!(text.starts_with("Intro\n") && text.ends_with("Figure 1: Chart\nOutro\n"));
    }

    /// FR-NEW-037 not broken: a picture reference kept as no image stays as
    /// written.
    #[tokio::test]
    async fn a_reference_kept_as_no_image_stays_verbatim() {
        let conversion = bundle(
            "![](missing.png)\n\n![](a.png)\n",
            &[Pic { target: "a.png", bytes: b"a", caption: "", page: None }],
        );
        let (_f, server) = converting_bundle("/m.pdf", conversion).await;
        convert(&server, "/m.pdf").await;
        let text = view(&server, "/m.pdf", true, None).await;
        assert!(text.starts_with("![](missing.png)\n"), "got: {text}");
        only_line(&text.lines().collect::<Vec<_>>(), "[Image image-2]");
    }

    /// SPEC-0019/E2E-064: the default view holds no picture reference or
    /// figure path, and every line outside image and table positions equals
    /// the conversion text, in order (FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_064_no_reference_or_figure_path_remains() {
        let conversion = report_bundle();
        let original = conversion.markdown.clone();
        let (_f, server) = converting_bundle("/report.pdf", conversion).await;
        convert(&server, "/report.pdf").await;
        let text = view(&server, "/report.pdf", true, None).await;
        assert!(!text.contains("![Figure]") && !text.contains("![Diagram]"), "got: {text}");
        assert!(
            !text.contains("figures/f1.png") && !text.contains("figures/f2.png"),
            "got: {text}"
        );
        for line in ["# Report", "Table 1: Q1 sales", "Table 2: Costs"] {
            assert!(text.contains(line), "missing {line:?}: {text}");
        }

        // Image positions (the 2 picture references) and table positions
        // (header, separator and rows, both tables) in the conversion text;
        // every other line must survive, unchanged and in order, in the view.
        let original_lines: Vec<&str> = original.lines().collect();
        let image_or_table_lines: std::collections::HashSet<usize> =
            [0usize, 19].into_iter().chain(6..=10).chain(14..=17).collect();
        let expected: Vec<&str> = original_lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !image_or_table_lines.contains(i))
            .map(|(_, l)| *l)
            .collect();
        let text_lines: Vec<&str> = text.lines().collect();
        let mut j = 0;
        for exp in &expected {
            while j < text_lines.len() && text_lines[j] != *exp {
                j += 1;
            }
            assert!(j < text_lines.len(), "line {exp:?} not found in order in: {text}");
            j += 1;
        }
    }

    /// SPEC-0019/E2E-065: after the sibling is replaced, the view is still
    /// built from the conversion text, not the sibling (FR-NEW-009).
    #[tokio::test]
    async fn spec_0019_e2e_065_a_rewritten_sibling_is_ignored() {
        let (f, server, _) = report_with_images().await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        client.write_bytes_atomic("/report.md", b"edited").await.unwrap();
        let text = view(&server, "/report.pdf", true, None).await;
        assert!(text.contains("[Image image-1: Figure 1: Revenue 2024]"), "got: {text}");
        assert!(!text.lines().any(|l| l == "edited"), "got: {text}");
    }

    /// SPEC-0019/E2E-066: after the sibling is deleted, the view is still
    /// built from the conversion text, in `csv-reference` mode (FR-NEW-009).
    #[tokio::test]
    async fn spec_0019_e2e_066_a_deleted_sibling_is_ignored() {
        let (f, server, _) = report_with_images().await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        client.delete_file("/report.md").await.unwrap();
        let text = view(&server, "/report.pdf", true, Some("csv-reference")).await;
        assert!(text.contains("[Table table-1: Q1 sales (CSV)]"), "got: {text}");
    }

    /// SPEC-0019/E2E-139: an image between plain text lines, with its caption
    /// right after, reads exactly the 5 described lines (FR-NEW-036).
    #[tokio::test]
    async fn spec_0019_e2e_139_an_image_between_text_lines_has_one_blank_each_side() {
        let conversion = bundle(
            "Intro\n![](a.png)\nFigure 1: Revenue 2024\nBody text",
            &[Pic { target: "a.png", bytes: b"a", caption: "Figure 1: Revenue 2024", page: None }],
        );
        let (_f, server) = converting_bundle("/fig.pdf", conversion).await;
        convert(&server, "/fig.pdf").await;
        let text = view(&server, "/fig.pdf", true, None).await;
        assert_eq!(
            text, "Intro\n\n[Image image-1: Figure 1: Revenue 2024]\n\nBody text",
            "got: {text}"
        );
    }

    /// SPEC-0019/E2E-144: an image written inside a line of other text stays
    /// as text, and is kept as no image (FR-NEW-033, FR-NEW-007).
    #[tokio::test]
    async fn spec_0019_e2e_144_an_inline_image_reference_stays_as_text() {
        let conversion = bundle(
            "See ![chart](figures/figure_1.png) here",
            &[Pic { target: "figures/figure_1.png", bytes: b"p", caption: "", page: None }],
        );
        let (_f, server) = converting_bundle("/inline.pdf", conversion).await;
        let out = convert(&server, "/inline.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");
        let text = view(&server, "/inline.pdf", true, None).await;
        assert_eq!(text, "See ![chart](figures/figure_1.png) here", "got: {text}");
    }

    /// SPEC-0019/E2E-159: a picture surrounded by empty lines on both sides,
    /// with its caption on its own line further surrounded by empty lines,
    /// collapses to exactly one empty line each side (FR-NEW-036).
    #[tokio::test]
    async fn spec_0019_e2e_159_surrounding_empty_lines_collapse_to_one_each_side() {
        let conversion = bundle(
            "Intro\n\n![](a.png)\n\nFigure 1: Revenue 2024\n\nBody text",
            &[Pic { target: "a.png", bytes: b"a", caption: "Figure 1: Revenue 2024", page: None }],
        );
        let (_f, server) = converting_bundle("/gap.pdf", conversion).await;
        convert(&server, "/gap.pdf").await;
        let text = view(&server, "/gap.pdf", true, None).await;
        assert_eq!(
            text, "Intro\n\n[Image image-1: Figure 1: Revenue 2024]\n\nBody text",
            "got: {text}"
        );
    }

    /// SPEC-0019/E2E-160: a picture reference as the first line of the
    /// conversion text has no empty line before its image line (FR-NEW-036).
    #[tokio::test]
    async fn spec_0019_e2e_160_a_first_line_image_has_no_leading_empty_line() {
        let conversion = bundle(
            "![](a.png)\n\nBody text",
            &[Pic { target: "a.png", bytes: b"a", caption: "", page: None }],
        );
        let (_f, server) = converting_bundle("/top.pdf", conversion).await;
        convert(&server, "/top.pdf").await;
        let text = view(&server, "/top.pdf", true, None).await;
        assert_eq!(text, "[Image image-1]\n\nBody text", "got: {text}");
    }

    /// SPEC-0019/E2E-161: a picture reference as the last line of the
    /// conversion text has no empty line after its image line (FR-NEW-036).
    #[tokio::test]
    async fn spec_0019_e2e_161_a_last_line_image_has_no_trailing_empty_line() {
        let conversion = bundle(
            "Intro\n\n\n![](a.png)",
            &[Pic { target: "a.png", bytes: b"a", caption: "", page: None }],
        );
        let (_f, server) = converting_bundle("/end.pdf", conversion).await;
        convert(&server, "/end.pdf").await;
        let text = view(&server, "/end.pdf", true, None).await;
        assert_eq!(text, "Intro\n\n[Image image-1]", "got: {text}");
    }

    /// SPEC-0019/E2E-162: a target escaping the converter's output is left
    /// unchanged, and its id renders no image line (FR-NEW-037).
    #[tokio::test]
    async fn spec_0019_e2e_162_a_traversal_target_stays_verbatim() {
        let secret = b"top secret pixels";
        let home = tempfile::tempdir().expect("a temp dir for the fake converter script");
        let sh = write_traversal_bundle_script(home.path(), secret);
        let cli = crate::docs::CliDocService::new(
            crate::config::DocServiceCliConfig {
                command: vec![
                    sh.display().to_string(),
                    crate::config::OUTDIR_PLACEHOLDER.into(),
                    crate::config::DOC_PLACEHOLDER.into(),
                ],
                timeout_secs: 30,
            },
            crate::docs::DOC_SERVICE_EXTS.iter().map(|e| (*e).to_string()).collect(),
            u64::MAX,
        );
        let (f, _) = setup(None).await;
        put(&f, "/trav.pdf", b"%PDF-1.4 source").await;
        let server = with_doc(&f, Some(Arc::new(cli)));
        convert(&server, "/trav.pdf").await;
        let text = view(&server, "/trav.pdf", true, None).await;
        assert!(text.lines().any(|l| l == "![x](../../secret.png)"), "got: {text}");
        assert!(text.contains("[Image image-2]"), "got: {text}");
        assert!(!text.lines().any(|l| l.starts_with("[Image image-1")), "got: {text}");
        let _ = std::fs::remove_file(std::env::temp_dir().join("secret.png"));
    }

    /// SPEC-0019/E2E-163: an unsupported picture format's reference line
    /// stands unchanged, and its id renders no image line (FR-NEW-037).
    #[tokio::test]
    async fn spec_0019_e2e_163_an_unsupported_format_reference_stays_verbatim() {
        let md = "![](a.jpg)\n\n![](b.svg)\n";
        let files: std::collections::BTreeMap<String, Vec<u8>> = [
            ("a.jpg".to_string(), b"jpeg bytes".to_vec()),
            ("b.svg".to_string(), b"<svg/>".to_vec()),
        ]
        .into();
        let conversion = Conversion { markdown: md.to_string(), files, manifest: None };
        let (_f, server) = converting_bundle("/mixed.pdf", conversion).await;
        convert(&server, "/mixed.pdf").await;
        let text = view(&server, "/mixed.pdf", true, None).await;
        assert!(text.lines().any(|l| l == "![](b.svg)"), "got: {text}");
        assert!(!text.lines().any(|l| l.starts_with("[Image image-2")), "got: {text}");
    }

    /// SPEC-0019/E2E-164: a URL target's reference line stands unchanged with
    /// captions off (FR-NEW-037).
    #[tokio::test]
    async fn spec_0019_e2e_164_a_url_reference_stays_verbatim_with_captions_off() {
        let md = "![x](https://example.com/a.png)\n";
        let conversion = Conversion {
            markdown: md.to_string(),
            files: std::collections::BTreeMap::new(),
            manifest: None,
        };
        let (_f, server) = converting_bundle("/url.pdf", conversion).await;
        convert(&server, "/url.pdf").await;
        let text = view(&server, "/url.pdf", false, None).await;
        assert!(text.lines().any(|l| l == "![x](https://example.com/a.png)"), "got: {text}");
    }

    // ── re-conversion replaces the set (US-0010) ────────────────────────────

    /// A fake converter that answers `conversion` only once the test releases
    /// it, after telling the test it started, so overlapping re-conversions
    /// interleave in an order the test decides, without sleeping.
    struct GatedDoc {
        conversion: Conversion,
        started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        /// Extensions and size cap, borrowed from the canned service.
        stub: StubDocService,
    }

    /// The gate's two ends: wait for the conversion to start, then let it end.
    struct Gate {
        started: tokio::sync::oneshot::Receiver<()>,
        release: tokio::sync::oneshot::Sender<()>,
    }

    fn gated(conversion: Conversion) -> (Arc<dyn DocService>, Gate) {
        let (started_tx, started) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let doc = GatedDoc {
            conversion,
            started: std::sync::Mutex::new(Some(started_tx)),
            release: tokio::sync::Mutex::new(Some(release_rx)),
            stub: StubDocService::ok(""),
        };
        (Arc::new(doc), Gate { started, release })
    }

    #[async_trait::async_trait]
    impl DocService for GatedDoc {
        async fn convert(
            &self,
            _bytes: &[u8],
            _file_name: &str,
        ) -> crate::errors::Result<Conversion> {
            let started = self.started.lock().unwrap().take();
            if let Some(tx) = started {
                let _ = tx.send(());
            }
            let release = self.release.lock().await.take();
            if let Some(rx) = release {
                let _ = rx.await;
            }
            Ok(self.conversion.clone())
        }

        fn accepted_extensions(&self) -> &[String] {
            self.stub.accepted_extensions()
        }

        fn max_input_bytes(&self) -> u64 {
            self.stub.max_input_bytes()
        }
    }

    const BOB: &str = "bob@test.com";

    /// `report.pdf`'s two tables with one PNG picture per caption before them.
    fn report_pictures(captions: &[&str]) -> Conversion {
        let targets: Vec<String> = (1..=captions.len()).map(|i| format!("n/{i}.png")).collect();
        let bytes: Vec<Vec<u8>> =
            captions.iter().map(|c| format!("png {c}").into_bytes()).collect();
        let mut md: String = targets.iter().map(|t| format!("![]({t})\n\n")).collect();
        md.push_str(REPORT_MD);
        let pics: Vec<Pic<'_>> = targets
            .iter()
            .zip(&bytes)
            .zip(captions)
            .map(|((t, b), c)| Pic { target: t, bytes: b, caption: c, page: Some(1) })
            .collect();
        bundle(&md, &pics)
    }

    async fn reconvert(server: &McpServer, path: &str) -> Value {
        ok_json(server.fs_documentize(Parameters(documentize_args(path, true))).await)
    }

    fn captions(list: &Value) -> Vec<String> {
        list["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["caption"].as_str().unwrap().to_string())
            .collect()
    }

    /// SPEC-0019/E2E-069: a re-conversion replaces every image, numbered from
    /// 1, and no former caption remains (FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_069_a_reconversion_replaces_and_renumbers_the_images() {
        let (f, _, _) = report_with_images().await;
        let server = bundling(&f, report_pictures(&["A", "B", "C"]));
        reconvert(&server, "/report.pdf").await;
        let list = images(&server, "/report.pdf", None).await;
        assert_eq!(image_ids(&list), numbered("image", 3));
        assert_eq!(captions(&list), vec!["A", "B", "C"]);
        assert!(!list.to_string().contains("Schéma réseau 🌐"), "got: {list}");
    }

    /// SPEC-0019/E2E-070: an extraction with refresh replaces the set with the
    /// built-in one, which finds no image and no PDF table (FR-NEW-026).
    #[tokio::test]
    async fn spec_0019_e2e_070_a_refreshed_extraction_keeps_the_builtin_set() {
        let (f, _) = setup(None).await;
        put(&f, "/report.pdf", &crate::docs::extract::tests::tiny_pdf()).await;
        let server = bundling(&f, report_bundle());
        assert_eq!(convert(&server, "/report.pdf").await["artifacts_report"], "2 images, 2 tables");
        let mut args = extract_args("/report.pdf", 200_000);
        args.refresh = true;
        ok_json(server.fs_extract_text(Parameters(args)).await);
        assert_eq!(images(&server, "/report.pdf", None).await, empty_images());
        assert_eq!(tables(&server, "/report.pdf").await, json!({"tables": [], "marker": null}));
    }

    /// The shared report's two images, two tables and sibling, unchanged.
    async fn report_unchanged(f: &Fixture, server: &McpServer, sibling: &str) {
        assert_eq!(
            images(server, "/report.pdf", None).await,
            json!({
                "images": [
                    {"id": "image-1", "caption": "Figure 1: Revenue 2024", "page": 1},
                    {"id": "image-2", "caption": "Schéma réseau 🌐", "page": 3},
                ],
                "marker": null,
            })
        );
        assert_eq!(ids(&tables(server, "/report.pdf").await), vec!["table-1", "table-2"]);
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert_eq!(client.read_text("/report.md").await.unwrap(), sibling);
    }

    async fn sibling(f: &Fixture) -> String {
        f.state.stores.client(PROJECT).await.unwrap().read_text("/report.md").await.unwrap()
    }

    /// SPEC-0019/E2E-072: an unreachable converter fails the re-conversion with
    /// its usual message and keeps the set and the sibling (FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_072_a_failed_reconversion_keeps_the_set() {
        let (f, _, _) = report_with_images().await;
        let before = sibling(&f).await;
        let message = "document service request failed: connection refused";
        let doc: Arc<dyn DocService> =
            Arc::new(StubDocService::failing(crate::errors::ToolError::internal(message)));
        let server = with_doc(&f, Some(doc));
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await,
        );
        assert!(err.contains(message), "got: {err}");
        report_unchanged(&f, &server, &before).await;
    }

    /// SPEC-0019/E2E-073: a convert action without overwrite is refused
    /// verbatim and keeps the former images (FR-NEW-027).
    #[tokio::test]
    async fn spec_0019_e2e_073_convert_without_overwrite_keeps_the_set() {
        let (f, _, _) = report_with_images().await;
        let before = sibling(&f).await;
        let server = bundling(&f, report_pictures(&["A", "B", "C"]));
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/report.pdf", false))).await,
        );
        assert!(err.contains("'/report.md' exists (pass overwrite=true)"), "got: {err}");
        report_unchanged(&f, &server, &before).await;
    }

    /// SPEC-0019/E2E-074: a non member re-converting gets the usual refusal
    /// and the set is unchanged (FR-NEW-016).
    #[tokio::test]
    async fn spec_0019_e2e_074_a_non_member_reconversion_keeps_the_set() {
        let (f, server, _) = report_with_images().await;
        let before = sibling(&f).await;
        let doc: Arc<dyn DocService> = Arc::new(StubDocService::bundle(report_pictures(&["X"])));
        let olga = with_doc_as(&f, Some(doc), "olga@test.com");
        let usual =
            err_text(olga.fs_extract_text(Parameters(extract_args("/report.pdf", 200_000))).await);
        let got =
            err_text(olga.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await);
        let refusal = |e: &str| e.split_once("': ").map(|(_, r)| r.to_string()).unwrap();
        assert!(got.contains("ERR_FORBIDDEN"), "got: {got}");
        assert_eq!(refusal(&got), refusal(&usual));
        report_unchanged(&f, &server, &before).await;
    }

    /// SPEC-0019/E2E-079: an id the new set no longer holds is not found
    /// (FR-NEW-010, FR-NEW-019).
    #[tokio::test]
    async fn spec_0019_e2e_079_a_dropped_id_is_not_found_after_reconversion() {
        let (f, _, _) = report_with_images().await;
        let server = bundling(&f, report_pictures(&["Only"]));
        reconvert(&server, "/report.pdf").await;
        let err = err_text(
            server.fs_get_image(Parameters(get_image_args("/report.pdf", "image-2"))).await,
        );
        assert!(err.contains("ERR_NOT_FOUND") && err.contains("Not found: image-2"), "got: {err}");
    }

    /// A project holding the converted `report.pdf` with Bob as a member.
    async fn report_with_bob() -> (Fixture, McpServer) {
        let (f, server, _) = report_with_images().await;
        f.state.admin.add_member(PROJECT, BOB, ALICE).await.unwrap();
        (f, server)
    }

    /// SPEC-0019/E2E-080: two overlapping re-conversions both finish and leave
    /// exactly one set, each id once (FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_080_overlapping_reconversions_leave_one_set() {
        let (f, server) = report_with_bob().await;
        let (alice_doc, alice_gate) = gated(report_bundle());
        let (bob_doc, bob_gate) = gated(report_bundle());
        let alice = with_doc_as(&f, Some(alice_doc), ALICE);
        let bob = with_doc_as(&f, Some(bob_doc), BOB);
        let drive = async {
            // Both conversions are in flight before either may finish.
            alice_gate.started.await.unwrap();
            bob_gate.started.await.unwrap();
            alice_gate.release.send(()).unwrap();
            bob_gate.release.send(()).unwrap();
        };
        let (a, b, ()) =
            tokio::join!(reconvert(&alice, "/report.pdf"), reconvert(&bob, "/report.pdf"), drive);
        assert_eq!(a["artifacts_report"], "2 images, 2 tables");
        assert_eq!(b["artifacts_report"], "2 images, 2 tables");
        assert_eq!(image_ids(&images(&server, "/report.pdf", None).await), numbered("image", 2));
    }

    /// SPEC-0019/E2E-081: a reader listing during a re-conversion sees the
    /// whole old set or the whole new one, never a mix (FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_081_a_reader_never_sees_a_mix() {
        let (f, server) = report_with_bob().await;
        let new = ["N1", "N2", "N3", "N4", "N5"];
        let (doc, gate) = gated(report_pictures(&new));
        let alice = with_doc(&f, Some(doc));
        let bob = with_doc_as(&f, None, BOB);
        let old = vec!["Figure 1: Revenue 2024".to_string(), "Schéma réseau 🌐".to_string()];
        let new: Vec<String> = new.iter().map(|c| (*c).to_string()).collect();
        let read = async {
            gate.started.await.unwrap();
            let mut release = Some(gate.release);
            let mut answers = Vec::new();
            for i in 0..20 {
                if i == 10 {
                    // Half the reads overlap the commit itself.
                    release.take().unwrap().send(()).unwrap();
                }
                answers.push(images(&bob, "/report.pdf", None).await);
                tokio::task::yield_now().await;
            }
            answers
        };
        let (_, answers) = tokio::join!(reconvert(&alice, "/report.pdf"), read);
        assert_eq!(answers.len(), 20);
        for list in &answers {
            let got = captions(list);
            assert!(got == old || got == new, "a mixed or partial answer: {list}");
            assert_eq!(image_ids(list), numbered("image", got.len()));
        }
        assert_eq!(captions(&images(&server, "/report.pdf", None).await), new);
    }

    /// SPEC-0019/E2E-082 and E2E-126: an extraction answered by the existing
    /// sibling keeps the set and carries no report (FR-NEW-026, FR-NEW-030).
    #[tokio::test]
    async fn spec_0019_e2e_082_126_a_cache_hit_keeps_the_set_and_reports_nothing() {
        let (f, server, _) = report_with_images().await;
        let before = sibling(&f).await;
        let out =
            ok_json(server.fs_extract_text(Parameters(extract_args("/report.pdf", 200_000))).await);
        assert!(out.get("artifacts_report").is_none(), "got: {out}");
        report_unchanged(&f, &server, &before).await;
    }

    /// SPEC-0019/E2E-083: convert with overwrite reports and keeps the new set
    /// only (FR-NEW-027).
    #[tokio::test]
    async fn spec_0019_e2e_083_convert_with_overwrite_replaces_the_set() {
        let (f, _, _) = report_with_images().await;
        let server = bundling(&f, report_pictures(&["Only"]));
        let out = reconvert(&server, "/report.pdf").await;
        assert_eq!(out["artifacts_report"], "1 images, 2 tables");
        assert_eq!(image_ids(&images(&server, "/report.pdf", None).await), vec!["image-1"]);
    }

    /// SPEC-0019/E2E-084: a documented upload of a new `report.pdf` discards
    /// the former set (FR-NEW-010, FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_084_a_documented_upload_replaces_the_set() {
        let (f, _, _) = report_with_images().await;
        let server = bundling(&f, report_pictures(&["New picture"]));
        let mut args = write_args("/report.pdf", b"%PDF-1.4 new source", true);
        args.overwrite = true;
        let out = ok_json(server.fs_write_bytes(Parameters(args)).await);
        assert_eq!(out["documentation"]["artifacts_report"], "1 images, 2 tables", "got: {out}");
        let list = images(&server, "/report.pdf", None).await;
        assert_eq!(image_ids(&list), vec!["image-1"]);
        assert_eq!(captions(&list), vec!["New picture"]);
        assert!(!list.to_string().contains("Schéma réseau 🌐"), "got: {list}");
    }

    /// SPEC-0019/E2E-136: a page marker past the end of the new set answers an
    /// empty page with no marker (FR-NEW-003).
    #[tokio::test]
    async fn spec_0019_e2e_136_a_marker_past_the_new_set_is_an_empty_page() {
        let (f, server) = converting_bundle("/atlas2.pdf", many_pictures(101)).await;
        convert(&server, "/atlas2.pdf").await;
        let first = images(&server, "/atlas2.pdf", None).await;
        let marker = first["marker"].as_str().expect("a marker").to_string();
        let server = bundling(&f, many_pictures(3));
        reconvert(&server, "/atlas2.pdf").await;
        assert_eq!(images(&server, "/atlas2.pdf", Some(&marker)).await, empty_images());
    }

    /// SPEC-0019/E2E-157: two overlapping re-conversions each report their own
    /// set; the one finishing last supplies the set and the sibling (FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_157_the_last_commit_supplies_set_and_sibling() {
        let (f, server) = report_with_bob().await;
        let bob_conversion = report_pictures(&["B1", "B2", "B3", "B4"]);
        let bob_markdown = bob_conversion.markdown.clone();
        let (alice_doc, alice_gate) = gated(report_pictures(&["A1", "A2", "A3"]));
        let (bob_doc, bob_gate) = gated(bob_conversion);
        let alice = with_doc_as(&f, Some(alice_doc), ALICE);
        let bob = with_doc_as(&f, Some(bob_doc), BOB);
        let (alice_done_tx, alice_done) = tokio::sync::oneshot::channel();
        let alice_call = async {
            let out = reconvert(&alice, "/report.pdf").await;
            alice_done_tx.send(()).unwrap();
            out
        };
        let drive = async {
            alice_gate.started.await.unwrap();
            bob_gate.started.await.unwrap();
            alice_gate.release.send(()).unwrap();
            alice_done.await.unwrap();
            bob_gate.release.send(()).unwrap();
        };
        let (a, b, ()) = tokio::join!(alice_call, reconvert(&bob, "/report.pdf"), drive);
        assert_eq!(a["artifacts_report"], "3 images, 2 tables");
        assert_eq!(b["artifacts_report"], "4 images, 2 tables");
        let list = images(&server, "/report.pdf", None).await;
        assert_eq!(image_ids(&list), numbered("image", 4));
        assert_eq!(captions(&list), vec!["B1", "B2", "B3", "B4"]);
        assert_eq!(sibling(&f).await, bob_markdown);
    }

    // ── refused and overtaken re-conversions (US-0011) ──────────────────────

    /// `report.pdf` (a parseable PDF, so the built-in extraction works too)
    /// converted through the fake converter under a session quota of `quota`.
    async fn report_with_quota(quota: i64) -> (Fixture, McpServer) {
        let (f, _) = setup(Some(quota)).await;
        put(&f, "/report.pdf", &crate::docs::extract::tests::tiny_pdf()).await;
        let server = bundling(&f, report_bundle());
        convert(&server, "/report.pdf").await;
        (f, server)
    }

    /// Charge Alice so exactly `left` bytes of her `quota` remain.
    fn leave(f: &Fixture, quota: i64, left: i64) {
        let used = f.state.safety.bytes_written(ALICE, PROJECT);
        f.state.safety.charge_write(ALICE, PROJECT, quota - left - used).unwrap();
    }

    /// A conversion with no picture whose sibling and conversion text weigh
    /// 25 000 bytes each: 50 000 together.
    fn heavy() -> Conversion {
        bundle(&"z".repeat(25_000), &[])
    }

    fn refresh_args(path: &str) -> ExtractTextArgs {
        let mut args = extract_args(path, 200_000);
        args.refresh = true;
        args
    }

    /// `person` reads `path` then overwrites it with `bytes`, the read
    /// before write guard being part of any real edit.
    async fn overwrite_as(server: &McpServer, path: &str, bytes: &[u8]) {
        let read = ReadBytesArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            offset_bytes: 0,
            length_bytes: 1_000_000,
        };
        ok_json(server.fs_read_bytes(Parameters(read)).await);
        let mut args = write_args(path, bytes, false);
        args.overwrite = true;
        ok_json(server.fs_write_bytes(Parameters(args)).await);
    }

    /// SPEC-0019/E2E-071: a re-conversion over the quota is refused and keeps
    /// the set and the sibling byte for byte (FR-NEW-013, FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_071_a_reconversion_over_the_quota_keeps_the_set() {
        let (f, _) = report_with_quota(QUOTA).await;
        let before = sibling(&f).await;
        leave(&f, QUOTA, 10_000);
        let server = bundling(&f, heavy());
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        report_unchanged(&f, &server, &before).await;
        assert_eq!(
            picture(&image(&server, "/report.pdf", "image-1").await),
            FIG_1,
            "the kept picture bytes"
        );
    }

    /// SPEC-0019/E2E-075: an extraction with refresh over the quota keeps the
    /// images (FR-MOD-001, FR-NEW-013, FR-NEW-026).
    #[tokio::test]
    async fn spec_0019_e2e_075_a_refused_refresh_keeps_the_images() {
        let (f, server) = report_with_quota(QUOTA).await;
        leave(&f, QUOTA, 10);
        let err = err_text(server.fs_extract_text(Parameters(refresh_args("/report.pdf"))).await);
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        assert_eq!(image_ids(&images(&server, "/report.pdf", None).await), numbered("image", 2));
    }

    /// SPEC-0019/E2E-076: an extraction with refresh over the quota leaves
    /// the sibling byte identical (FR-MOD-001).
    #[tokio::test]
    async fn spec_0019_e2e_076_a_refused_refresh_keeps_the_sibling() {
        let (f, server) = report_with_quota(QUOTA).await;
        let reference = sibling(&f).await;
        leave(&f, QUOTA, 10);
        let err = err_text(server.fs_extract_text(Parameters(refresh_args("/report.pdf"))).await);
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        assert_eq!(sibling(&f).await, reference);
    }

    /// SPEC-0019/E2E-077: a re-conversion over the quota leaves the sibling
    /// byte identical and the images unchanged (FR-NEW-013, FR-NEW-022).
    #[tokio::test]
    async fn spec_0019_e2e_077_a_refused_reconversion_keeps_sibling_and_images() {
        let (f, server) = report_with_quota(QUOTA).await;
        let reference = sibling(&f).await;
        leave(&f, QUOTA, 10);
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
        report_unchanged(&f, &server, &reference).await;
    }

    /// SPEC-0019/E2E-078: a documented upload whose conversion overflows the
    /// quota keeps the new source, reports the quota message, and leaves the
    /// sibling as it was (FR-NEW-014, FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_078_a_documented_upload_over_the_quota_keeps_the_sibling() {
        const BIG: i64 = 600_000;
        let (f, _) = report_with_quota(BIG).await;
        let reference = sibling(&f).await;
        leave(&f, BIG, 300_000);
        let server = bundling(&f, heavy());
        let source = vec![b'p'; 290_000];
        let mut args = write_args("/report.pdf", &source, true);
        args.overwrite = true;
        let out = ok_json(server.fs_write_bytes(Parameters(args)).await);
        assert_eq!(
            out["documentation"]["error"]["message"],
            "session write quota of 600000 bytes exceeded",
            "got: {out}"
        );
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert_eq!(client.read_bytes("/report.pdf").await.unwrap(), source);
        assert_eq!(sibling(&f).await, reference);
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-085: a re-conversion keeps its full charge, the former
    /// set's bytes are not given back (FR-NEW-013).
    #[tokio::test]
    async fn spec_0019_e2e_085_a_reconversion_gives_no_quota_back() {
        let (f, _) = report_with_quota(QUOTA).await;
        leave(&f, QUOTA, 60_000);
        let server = bundling(&f, heavy());
        let out = reconvert(&server, "/report.pdf").await;
        assert_eq!(out["artifacts_report"], "0 images, 0 tables");
        let err = err_text(
            server.fs_write_bytes(Parameters(write_args("/w.txt", &[b'w'; 20_000], false))).await,
        );
        assert!(err.contains("session write quota of 262144 bytes exceeded"), "got: {err}");
    }

    /// SPEC-0019/E2E-107: with the converter removed the convert action is
    /// refused and the set is kept (FR-NEW-029, FR-NEW-010).
    #[tokio::test]
    async fn spec_0019_e2e_107_no_converter_keeps_the_set() {
        let (f, _, _) = report_with_images().await;
        let server = with_doc(&f, None);
        let err = err_text(
            server.fs_documentize(Parameters(documentize_args("/report.pdf", true))).await,
        );
        assert!(err.contains("document service is not configured"), "got: {err}");
        assert_eq!(image_ids(&images(&server, "/report.pdf", None).await), numbered("image", 2));
    }

    fn assert_changed(err: &str, path: &str) {
        assert!(err.contains(&format!("Document changed during conversion: {path}")), "got: {err}");
        assert!(err.contains("ERR_INVALID_ARGUMENT"), "invalid argument category, got: {err}");
    }

    /// SPEC-0019/E2E-133: a document rewritten during its re-conversion
    /// refuses it, and the document is then not extracted (FR-NEW-035).
    #[tokio::test]
    async fn spec_0019_e2e_133_a_write_during_reconversion_refuses_it() {
        let (f, _) = report_with_bob().await;
        let reference = sibling(&f).await;
        let (doc, gate) = gated(report_pictures(&["A", "B", "C"]));
        let alice = with_doc(&f, Some(doc));
        let bob = with_doc_as(&f, None, BOB);
        let drive = async {
            gate.started.await.unwrap();
            overwrite_as(&bob, "/report.pdf", b"%PDF-1.4 bob").await;
            gate.release.send(()).unwrap();
        };
        let (got, ()) = tokio::join!(
            alice.fs_documentize(Parameters(documentize_args("/report.pdf", true))),
            drive
        );
        assert_changed(&err_text(got), "/report.pdf");
        no_artifacts(&alice, "/report.pdf").await;
        assert_eq!(sibling(&f).await, reference);
    }

    /// SPEC-0019/E2E-134: a document moved during its first conversion
    /// refuses it, and neither path keeps artifacts or a sibling (FR-NEW-035).
    #[tokio::test]
    async fn spec_0019_e2e_134_a_move_during_conversion_refuses_it() {
        let (f, _) = setup(None).await;
        f.state.admin.add_member(PROJECT, BOB, ALICE).await.unwrap();
        put(&f, "/fresh.pdf", b"%PDF-1.4 fresh").await;
        let (doc, gate) = gated(report_bundle());
        let alice = with_doc(&f, Some(doc));
        let bob = with_doc_as(&f, None, BOB);
        let drive = async {
            gate.started.await.unwrap();
            let args = MoveArgs {
                mount_id: PROJECT.into(),
                source: "/fresh.pdf".into(),
                destination: "/moved.pdf".into(),
                overwrite: false,
            };
            ok_json(bob.fs_move(Parameters(args)).await);
            gate.release.send(()).unwrap();
        };
        let (got, ()) = tokio::join!(
            alice.fs_documentize(Parameters(documentize_args("/fresh.pdf", false))),
            drive
        );
        assert_changed(&err_text(got), "/fresh.pdf");
        no_artifacts(&alice, "/moved.pdf").await;
        let client = f.state.stores.client(PROJECT).await.unwrap();
        assert!(!client.exists("/fresh.md").await.unwrap(), "no sibling kept");
    }

    /// SPEC-0019/E2E-135: a read during a re-conversion changes nothing, the
    /// re-conversion succeeds (FR-NEW-035).
    #[tokio::test]
    async fn spec_0019_e2e_135_a_read_during_reconversion_lets_it_succeed() {
        let (f, _) = report_with_bob().await;
        let (doc, gate) = gated(report_pictures(&["A", "B", "C"]));
        let alice = with_doc(&f, Some(doc));
        let bob = with_doc_as(&f, None, BOB);
        let drive = async {
            gate.started.await.unwrap();
            let read = ReadBytesArgs {
                mount_id: PROJECT.into(),
                path: "/report.pdf".into(),
                offset_bytes: 0,
                length_bytes: 1_000,
            };
            ok_json(bob.fs_read_bytes(Parameters(read)).await);
            gate.release.send(()).unwrap();
        };
        let (out, ()) = tokio::join!(reconvert(&alice, "/report.pdf"), drive);
        assert_eq!(out["artifacts_report"], "3 images, 2 tables");
        assert_eq!(captions(&images(&alice, "/report.pdf", None).await), vec!["A", "B", "C"]);
    }

    /// SPEC-0019/E2E-138: an overtaken re-conversion charges nothing and
    /// leaves the sibling byte identical (FR-NEW-035, FR-NEW-013).
    #[tokio::test]
    async fn spec_0019_e2e_138_an_overtaken_reconversion_charges_nothing() {
        let (f, _) = report_with_quota(QUOTA).await;
        f.state.admin.add_member(PROJECT, BOB, ALICE).await.unwrap();
        let reference = sibling(&f).await;
        let used = f.state.safety.bytes_written(ALICE, PROJECT);
        f.state.safety.refund_write(ALICE, PROJECT, used);
        let (doc, gate) = gated(report_pictures(&["A", "B", "C"]));
        let alice = with_doc(&f, Some(doc));
        let bob = with_doc_as(&f, None, BOB);
        let drive = async {
            gate.started.await.unwrap();
            overwrite_as(&bob, "/report.pdf", b"%PDF-1.4 bob").await;
            gate.release.send(()).unwrap();
        };
        let (got, ()) = tokio::join!(
            alice.fs_documentize(Parameters(documentize_args("/report.pdf", true))),
            drive
        );
        assert_changed(&err_text(got), "/report.pdf");
        assert_eq!(sibling(&f).await, reference);
        ok_json(
            alice.fs_write_bytes(Parameters(write_args("/n.txt", &[b'n'; 262_144], false))).await,
        );
    }

    /// FR-NEW-035 on the extraction path: a write landing while an extraction
    /// with refresh waits for the document's artifact lock refuses it,
    /// charges nothing and leaves the sibling and the set as they were.
    #[tokio::test]
    async fn an_extraction_overtaken_by_a_write_charges_nothing() {
        let (f, server) = report_with_quota(QUOTA).await;
        f.state.admin.add_member(PROJECT, BOB, ALICE).await.unwrap();
        let reference = sibling(&f).await;
        let used = f.state.safety.bytes_written(ALICE, PROJECT);
        let bob = with_doc_as(&f, None, BOB);
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let store = client.trash.clone().expect("a relational store");
        let held = store.lock_artifacts("/report.pdf").await;
        let drive = async {
            // The extraction converted and now waits on the held lock.
            while store.artifact_lock_contenders("/report.pdf") < 2 {
                tokio::task::yield_now().await;
            }
            overwrite_as(&bob, "/report.pdf", &crate::docs::extract::tests::tiny_pdf()).await;
            drop(held);
        };
        let (got, ()) =
            tokio::join!(server.fs_extract_text(Parameters(refresh_args("/report.pdf"))), drive);
        assert_changed(&err_text(got), "/report.pdf");
        assert_eq!(sibling(&f).await, reference);
        assert_eq!(f.state.safety.bytes_written(ALICE, PROJECT), used, "nothing charged");
        no_artifacts(&server, "/report.pdf").await;
    }

    // ── changed documents lose their artifacts (US-0012) ───────────────────

    fn report_ids(list: &Value) {
        assert_eq!(image_ids(list), numbered("image", 2));
    }

    async fn read(server: &McpServer, path: &str) {
        let args = ReadBytesArgs {
            mount_id: PROJECT.into(),
            path: path.into(),
            offset_bytes: 0,
            length_bytes: 1_000_000,
        };
        ok_json(server.fs_read_bytes(Parameters(args)).await);
    }

    async fn move_to(server: &McpServer, source: &str, destination: &str) {
        let args = MoveArgs {
            mount_id: PROJECT.into(),
            source: source.into(),
            destination: destination.into(),
            overwrite: false,
        };
        ok_json(server.fs_move(Parameters(args)).await);
    }

    async fn copy_to(server: &McpServer, source: &str, destination: &str, overwrite: bool) {
        let args = CopyArgs {
            mount_id: PROJECT.into(),
            source: source.into(),
            destination: destination.into(),
            overwrite,
            recursive: false,
        };
        ok_json(server.fs_copy(Parameters(args)).await);
    }

    /// SPEC-0019/E2E-086: reading the document, listing its folder and
    /// reading its sibling keep the artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_086_reads_keep_the_artifacts() {
        let (_f, server, _) = report_with_images().await;
        read(&server, "/report.pdf").await;
        let list = ListDirArgs {
            mount_id: PROJECT.into(),
            path: "/".into(),
            include_hidden: false,
            sort_by: "name".into(),
            with_sizes: true,
        };
        ok_json(server.fs_list_dir(Parameters(list)).await);
        read(&server, "/report.md").await;
        report_ids(&images(&server, "/report.pdf", None).await);
    }

    /// SPEC-0019/E2E-087: a moved document has no artifacts until it is
    /// converted again at its new path (FR-NEW-011, FR-NEW-001).
    #[tokio::test]
    async fn spec_0019_e2e_087_a_moved_document_converts_again() {
        let (_f, server, _) = report_with_images().await;
        move_to(&server, "/report.pdf", "/archive/r.pdf").await;
        no_artifacts(&server, "/archive/r.pdf").await;
        convert(&server, "/archive/r.pdf").await;
        report_ids(&images(&server, "/archive/r.pdf", None).await);
    }

    /// SPEC-0019/E2E-088: after a move the new path has no artifacts and the
    /// old one is not a file (FR-NEW-011, FR-NEW-028).
    #[tokio::test]
    async fn spec_0019_e2e_088_a_move_leaves_no_artifacts_on_either_path() {
        let (_f, server, _) = report_with_images().await;
        move_to(&server, "/report.pdf", "/b.pdf").await;
        no_artifacts(&server, "/b.pdf").await;
        let err = err_text(
            server.fs_list_images(Parameters(list_images_args("/report.pdf", None))).await,
        );
        assert!(err.contains("not a file: /report.pdf"), "got: {err}");
    }

    /// SPEC-0019/E2E-089: a copy starts with no artifacts, the source keeps
    /// its own (FR-NEW-012).
    #[tokio::test]
    async fn spec_0019_e2e_089_a_copy_has_no_artifacts() {
        let (_f, server, _) = report_with_images().await;
        copy_to(&server, "/report.pdf", "/copy.pdf", false).await;
        no_artifacts(&server, "/copy.pdf").await;
        report_ids(&images(&server, "/report.pdf", None).await);
    }

    /// FR-NEW-012: the copy is charged the file bytes only, never the
    /// source's artifacts.
    #[tokio::test]
    async fn a_copy_is_charged_the_file_bytes_only() {
        let (f, server, _) = report_with_images().await;
        let before = f.state.safety.bytes_written(ALICE, PROJECT);
        copy_to(&server, "/report.pdf", "/copy.pdf", false).await;
        let size = b"%PDF-1.4 source".len() as i64;
        assert_eq!(f.state.safety.bytes_written(ALICE, PROJECT) - before, size);
    }

    /// SPEC-0019/E2E-090: a document deleted to the trash and restored has
    /// no artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_090_trash_and_restore_discards_the_artifacts() {
        let (_f, server, _) = report_with_images().await;
        let args = DeleteArgs {
            mount_id: PROJECT.into(),
            path: "/report.pdf".into(),
            recursive: false,
            trash: true,
        };
        let out = ok_json(server.fs_delete(Parameters(args)).await);
        let trash_path = out["trash_path"].as_str().expect("a trash path").to_string();
        let restore = TrashRestoreArgs { mount_id: PROJECT.into(), trash_path };
        ok_json(server.fs_trash_restore(Parameters(restore)).await);
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-091: writing the same bytes again discards the
    /// artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_091_the_same_bytes_again_discard_the_artifacts() {
        let (_f, server, _) = report_with_images().await;
        overwrite_as(&server, "/report.pdf", b"%PDF-1.4 source").await;
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-093: another document copied over the converted one
    /// leaves no artifacts (FR-NEW-011, FR-NEW-012).
    #[tokio::test]
    async fn spec_0019_e2e_093_a_copy_over_the_document_discards_the_artifacts() {
        let (f, server, _) = report_with_images().await;
        put(&f, "/notes.pdf", b"%PDF-1.4 notes").await;
        copy_to(&server, "/notes.pdf", "/report.pdf", true).await;
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-094: an archive extracted over the document discards
    /// its artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_094_an_extracted_archive_discards_the_artifacts() {
        let (f, server, _) = report_with_images().await;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("report.pdf", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, b"%PDF-1.4 archived").unwrap();
        put(&f, "/pack.zip", &zip.finish().unwrap().into_inner()).await;
        let args = ExtractArchiveArgs {
            mount_id: PROJECT.into(),
            path: "/pack.zip".into(),
            destination: Some("/".into()),
            overwrite: true,
            password: None,
        };
        ok_json(server.fs_extract_archive(Parameters(args)).await);
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-095: moving the folder holding the document discards
    /// its artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_095_a_moved_folder_discards_the_artifacts() {
        let (f, server) = converting_bundle("/q1/report.pdf", report_bundle()).await;
        convert(&server, "/q1/report.pdf").await;
        report_ids(&images(&server, "/q1/report.pdf", None).await);
        move_to(&server, "/q1", "/archive").await;
        no_artifacts(&server, "/archive/report.pdf").await;
        drop(f);
    }

    /// SPEC-0019/E2E-096: rewriting the sibling leaves the document and its
    /// artifacts alone (FR-NEW-011, FR-NEW-022).
    #[tokio::test]
    async fn spec_0019_e2e_096_rewriting_the_sibling_keeps_the_artifacts() {
        let (_f, server, _) = report_with_images().await;
        overwrite_as(&server, "/report.md", b"edited\n").await;
        report_ids(&images(&server, "/report.pdf", None).await);
    }

    /// SPEC-0019/E2E-097: a move away and back discards the artifacts
    /// (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_097_away_and_back_discards_the_artifacts() {
        let (_f, server, _) = report_with_images().await;
        move_to(&server, "/report.pdf", "/b.pdf").await;
        move_to(&server, "/b.pdf", "/report.pdf").await;
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-108: an in place edit discards the artifacts
    /// (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_108_an_edit_discards_the_artifacts() {
        let (_f, server, _) = report_with_images().await;
        read(&server, "/report.pdf").await;
        let args = EditArgs {
            mount_id: PROJECT.into(),
            path: "/report.pdf".into(),
            old_string: "source".into(),
            new_string: "edited".into(),
            replace_all: false,
            dry_run: false,
        };
        ok_json(server.fs_edit(Parameters(args)).await);
        no_artifacts(&server, "/report.pdf").await;
    }

    /// SPEC-0019/E2E-137: a project soft deleted then restored keeps the
    /// artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_137_a_restored_project_keeps_the_artifacts() {
        let (f, server, _) = report_with_images().await;
        assert!(f.state.admin.soft_delete_project(PROJECT).await.unwrap());
        let args = ProjectIdArgs { project_id: PROJECT.into() };
        ok_json(server.admin_undelete_project(Parameters(args)).await);
        report_ids(&images(&server, "/report.pdf", None).await);
    }

    // The `git.*` tool methods reach the process wide repository store, fixed
    // to the first caller's configuration, whose temporary directory another
    // test may already have dropped. The git operations below therefore run
    // the same `tool_*` functions with a store of the test's own, the way the
    // git module's tests do; conversion and listing stay on `McpServer`.

    struct Git {
        f: Fixture,
        server: McpServer,
        store: Arc<crate::git::GitRepoStore>,
        mount: &'static str,
    }

    impl Git {
        async fn new(mount: &'static str) -> Git {
            let f = Fixture::with_config(|c| c.git.enabled = true).await;
            f.seed_project(mount, ALICE).await;
            let server = bundling(&f, report_bundle());
            let store = Arc::new(crate::git::GitRepoStore::new(
                f.state.config.clone(),
                f.state.stores.relational().clone(),
            ));
            Git { f, server, store, mount }
        }

        fn ctx(&self) -> crate::tools::registry_support::ToolCtx {
            self.f.ctx(ALICE)
        }

        fn g(&self) -> Option<Arc<crate::git::GitRepoStore>> {
            Some(self.store.clone())
        }

        async fn init(&self) {
            let args = MountOnlyArgs { mount_id: self.mount.into() };
            crate::tools::git::tool_init(self.ctx(), args, self.g()).await.unwrap();
        }

        async fn put(&self, path: &str, bytes: &[u8]) {
            let client = self.f.state.stores.client(self.mount).await.unwrap();
            client.write_bytes_atomic(path, bytes).await.unwrap();
        }

        async fn bytes(&self, path: &str) -> Vec<u8> {
            self.f.state.stores.client(self.mount).await.unwrap().read_bytes(path).await.unwrap()
        }

        async fn convert(&self, path: &str) {
            let args =
                DocumentizeArgs { mount_id: self.mount.into(), path: path.into(), overwrite: true };
            ok_json(self.server.fs_documentize(Parameters(args)).await);
        }

        async fn images(&self, path: &str) -> Result<CallToolResult, rmcp::ErrorData> {
            let args =
                ListImagesArgs { mount_id: self.mount.into(), path: path.into(), marker: None };
            self.server.fs_list_images(Parameters(args)).await
        }

        async fn commit(&self, message: &str) -> String {
            let args = GitCommitArgs {
                mount_id: self.mount.into(),
                message: message.into(),
                author_name: None,
                author_email: None,
            };
            let out = crate::tools::git::tool_commit(self.ctx(), args, self.g()).await.unwrap();
            out["commit_sha"].as_str().expect("a commit sha").to_string()
        }

        async fn switch(&self, name: &str) {
            let args = GitBranchSwitchArgs { mount_id: self.mount.into(), name: name.into() };
            crate::tools::git::tool_branch_switch(self.ctx(), args, self.g()).await.unwrap();
        }
    }

    fn assert_no_artifacts(r: Result<CallToolResult, rmcp::ErrorData>) {
        let err = err_text(r);
        assert!(err.contains("No artifacts: document not extracted"), "got: {err}");
    }

    /// A bare repository whose `main` holds `files`, as a `file://` url.
    fn bare_remote(dir: &std::path::Path, files: &[(&str, &[u8])]) -> String {
        let repo = git2::Repository::init_bare(dir).unwrap();
        commit_bare(&repo, files, &[]);
        repo.set_head("refs/heads/main").unwrap();
        format!("file://{}", dir.display())
    }

    /// One commit on `main` of `repo` whose tree is `files` plus the parent's.
    fn commit_bare(
        repo: &git2::Repository,
        files: &[(&str, &[u8])],
        parents: &[&git2::Commit<'_>],
    ) {
        let base = parents.first().map(|p| p.tree().unwrap());
        let mut builder = repo.treebuilder(base.as_ref()).unwrap();
        for (name, bytes) in files {
            builder.insert(*name, repo.blob(bytes).unwrap(), 0o100_644).unwrap();
        }
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let sig =
            git2::Signature::new("Origin", "o@t.com", &git2::Time::new(1_700_000_000, 0)).unwrap();
        repo.commit(Some("refs/heads/main"), &sig, &sig, "remote\n", &tree, parents).unwrap();
    }

    /// SPEC-0019/E2E-092: a pull bringing a changed document discards its
    /// artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_092_a_pull_changing_the_document_discards_the_artifacts() {
        let git = Git::new("artifacts-pull").await;
        let dir = git.f.dir.path().join("remote-092");
        let url = bare_remote(&dir, &[("report.pdf", b"%PDF-1.4 source")]);
        crate::tools::git::test_support::clone_from(&git.ctx(), git.store.clone(), git.mount, &url)
            .await
            .unwrap();
        git.convert("/report.pdf").await;
        git.commit("sibling").await;
        report_ids(&ok_json(git.images("/report.pdf").await));
        let remote = git2::Repository::open_bare(&dir).unwrap();
        let tip = remote.find_reference("refs/heads/main").unwrap().peel_to_commit().unwrap();
        commit_bare(&remote, &[("report.pdf", b"%PDF-1.4 changed")], &[&tip]);
        crate::tools::git::test_support::pull_from(&git.ctx(), &git.store, git.mount, &url, "main")
            .await
            .unwrap();
        assert_eq!(git.bytes("/report.pdf").await, b"%PDF-1.4 changed");
        assert_no_artifacts(git.images("/report.pdf").await);
    }

    /// SPEC-0019/E2E-109: restoring the committed version over a converted,
    /// different current version discards the artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_109_a_file_restore_discards_the_artifacts() {
        let git = Git::new("artifacts-restore").await;
        git.init().await;
        git.put("/report.pdf", b"%PDF-1.4 committed").await;
        let committed = git.commit("c0").await;
        git.put("/report.pdf", b"%PDF-1.4 current").await;
        git.convert("/report.pdf").await;
        report_ids(&ok_json(git.images("/report.pdf").await));
        let args = GitCheckoutFileArgs {
            mount_id: git.mount.into(),
            commit_sha: committed,
            path: "/report.pdf".into(),
        };
        crate::tools::git::tool_checkout_file(git.ctx(), args, git.g()).await.unwrap();
        assert_eq!(git.bytes("/report.pdf").await, b"%PDF-1.4 committed");
        assert_no_artifacts(git.images("/report.pdf").await);
    }

    /// SPEC-0019/E2E-141: a checkout leaves a document identical on both
    /// branches untouched, so it keeps its artifacts (FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_141_an_identical_document_survives_a_checkout() {
        let git = Git::new("artifacts-checkout").await;
        git.init().await;
        git.put("/report.pdf", b"%PDF-1.4 source").await;
        git.put("/other.txt", b"main\n").await;
        git.convert("/report.pdf").await;
        let c0 = git.commit("c0").await;
        let create = GitBranchCreateArgs {
            mount_id: git.mount.into(),
            name: "dev".into(),
            start_point: c0,
            checkout: true,
        };
        crate::tools::git::tool_branch_create(git.ctx(), create, git.g()).await.unwrap();
        git.put("/other.txt", b"dev\n").await;
        git.commit("c1").await;
        git.switch("main").await;
        assert_eq!(git.bytes("/other.txt").await, b"main\n");
        git.switch("dev").await;
        assert_eq!(git.bytes("/other.txt").await, b"dev\n");
        report_ids(&ok_json(git.images("/report.pdf").await));
    }

    /// Every entry name a commit's tree holds, walked recursively.
    async fn committed_tree_entries(git: &Git, sha: &str) -> Vec<String> {
        let entry = git.store.get_or_open_repo(git.mount).await.unwrap();
        let repo = entry.repo.lock().await;
        let commit = repo.find_commit(git2::Oid::from_str(sha).unwrap()).unwrap();
        let tree = commit.tree().unwrap();
        let mut names = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |_, entry| {
            names.push(entry.name().unwrap_or("").to_string());
            git2::TreeWalkResult::Ok
        })
        .unwrap();
        names
    }

    /// SPEC-0019/E2E-019: a folder listing and a pushed commit's tree both
    /// show `report.pdf` and `report.md` only, no image or table of it
    /// (FR-NEW-021). "Pushed" is exercised as "committed": the pushed history
    /// is exactly the committed tree a push transfers, built from `nodes`
    /// alone (DEC-002), so a commit already proves what a push would carry.
    #[tokio::test]
    async fn spec_0019_e2e_019_artifacts_are_absent_from_listings_and_pushed_history() {
        let git = Git::new("artifacts-019").await;
        git.init().await;
        git.put("/report.pdf", b"%PDF-1.4 source").await;
        git.convert("/report.pdf").await;
        report_ids(&ok_json(git.images("/report.pdf").await));
        let sha = git.commit("convert report").await;

        let list = ListDirArgs {
            mount_id: git.mount.into(),
            path: "/".into(),
            include_hidden: false,
            sort_by: "name".into(),
            with_sizes: true,
        };
        let out = ok_json(git.server.fs_list_dir(Parameters(list)).await);
        let names: Vec<String> = out["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["report.md", "report.pdf"], "folder listing: {names:?}");

        let mut tree_names = committed_tree_entries(&git, &sha).await;
        tree_names.sort();
        assert_eq!(tree_names, vec!["report.md", "report.pdf"], "pushed history: {tree_names:?}");
    }

    /// SPEC-0019/E2E-022: a file name search and a content search both skip
    /// every image and table, surfacing only real files (FR-NEW-021).
    #[tokio::test]
    async fn spec_0019_e2e_022_artifacts_are_absent_from_searches() {
        let (_f, server, _) = report_with_images().await;

        let out = ok_json(
            server
                .fs_glob(Parameters(GlobArgs {
                    mount_id: PROJECT.into(),
                    pattern: "**/*".into(),
                    root: "/".into(),
                    exclude_patterns: ExcludePatterns(Vec::new()),
                }))
                .await,
        );
        let mut matches: Vec<String> = out["matches"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default();
        matches.sort();
        assert_eq!(matches, vec!["/report.md", "/report.pdf"], "glob: {matches:?}");

        let out = ok_json(
            server
                .fs_grep(Parameters(GrepArgs {
                    mount_id: PROJECT.into(),
                    pattern: "Q1 sales".into(),
                    root: "/".into(),
                    include_glob: None,
                    exclude_glob: None,
                    regex: false,
                    case_sensitive: true,
                    output_mode: "files".into(),
                    context_lines: 0,
                    max_matches: 100,
                }))
                .await,
        );
        let files: Vec<String> = out["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(files, vec!["/report.md"], "content search: {files:?}");
    }

    /// SPEC-0019/E2E-023: a project export zip holds `report.pdf` and
    /// `report.md` only (FR-NEW-021).
    #[tokio::test]
    async fn spec_0019_e2e_023_artifacts_are_absent_from_exports() {
        let (f, server, _) = report_with_images().await;
        let out = ok_json(
            server
                .fs_export_zip(Parameters(ExportZipArgs {
                    mount_id: PROJECT.into(),
                    paths: vec!["/".to_string()],
                }))
                .await,
        );
        let url = out["url"].as_str().unwrap();
        let token = url.rsplit('/').next().unwrap();
        let client = f.state.stores.client(PROJECT).await.unwrap();
        let bytes = client.blob.get(&format!("export:{token}"), 0, None).await.unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> =
            (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["report.md", "report.pdf"], "export zip: {names:?}");
    }

    /// SPEC-0019/E2E-098: the trash listing after deleting the document shows
    /// `report.pdf` only, no image or table entry (FR-NEW-021, FR-NEW-011).
    #[tokio::test]
    async fn spec_0019_e2e_098_artifacts_are_absent_from_trash_listing() {
        let (f, server, _) = report_with_images().await;
        let delete = DeleteArgs {
            mount_id: PROJECT.into(),
            path: "/report.pdf".into(),
            recursive: false,
            trash: true,
        };
        ok_json(server.fs_delete(Parameters(delete)).await);

        let out = crate::tools::trash::trash_list(&f.state, PROJECT, "", 200, 0).await.unwrap();
        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "got: {entries:?}");
        assert_eq!(entries[0]["original_path"], json!("/report.pdf"));
    }

    /// SPEC-0019/E2E-099: copying a converted document charges only its file
    /// bytes, never its artifact bytes (FR-NEW-012, FR-NEW-013).
    #[tokio::test]
    async fn spec_0019_e2e_099_a_copy_charges_only_the_file_bytes() {
        let (f, _) = setup(Some(1_000_000)).await;
        let pixels = vec![7u8; 200_000];
        let conversion = bundle(
            "# Small\n![Figure](figures/f1.png)\n",
            &[Pic { target: "figures/f1.png", bytes: &pixels, caption: "Figure", page: None }],
        );
        let server = bundling(&f, conversion);
        let small = vec![9u8; 5_000];
        put(&f, "/small.pdf", &small).await;
        convert(&server, "/small.pdf").await;

        // Pin the remaining session quota to exactly 10 000 bytes, whatever
        // the setup write and the conversion itself already charged.
        f.state.safety.refund_write(ALICE, PROJECT, 10_000_000);
        f.state.safety.charge_write(ALICE, PROJECT, 990_000).unwrap();

        copy_to(&server, "/small.pdf", "/c.pdf", false).await;
        let write = write_args("/new.bin", &vec![1u8; 4_000], false);
        ok_json(server.fs_write_bytes(Parameters(write)).await);

        // The copy charged only `small.pdf`'s 5 000 file bytes (never the
        // artifact bytes), so 10 000 - 5 000 - 4 000 leaves exactly 1 000
        // bytes of session quota remaining: a 1 000 byte write succeeds, and
        // one more byte is refused.
        let fits = write_args("/fits.bin", &vec![2u8; 1_000], false);
        ok_json(server.fs_write_bytes(Parameters(fits)).await);

        let overflow = write_args("/overflow.bin", &[3u8; 1], false);
        let result = server.fs_write_bytes(Parameters(overflow)).await.unwrap();
        assert!(result.is_error.unwrap_or(false), "the next byte must be refused: {result:?}");
        assert!(
            text_of(&result).contains("write quota"),
            "the next byte must be refused for a quota reason: {result:?}"
        );
    }
}
