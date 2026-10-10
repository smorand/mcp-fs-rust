//! Document artifacts (SPEC-0019): the tables a conversion kept, listed per
//! document and read one by one.
//!
//! Typed functions only: the `#[tool]` methods in `mcp::server` authorize and
//! normalize the path, then call straight through to [`list_tables`] and
//! [`get_table`], so every door shares this one implementation (DEC-001).

use crate::docs::artifacts::render::TableFormat;
use crate::docs::artifacts::{
    ARTIFACT_PAGE_SIZE, CsvQuality, ListKind, NO_ARTIFACTS, decode_marker, encode_marker,
};
use crate::errors::{Result, ToolError};
use crate::state::AppState;
#[cfg(test)]
use crate::tools::authorize_only;
#[cfg(test)]
use crate::tools::registry_support::{ToolRegistry, ToolSchema, handler};
use serde_json::{Value, json};

/// Test-dispatch glue for `fs.list_tables`, mirroring the production handler
/// (`McpServer::fs_list_tables`) so the golden contract sees the same schema.
#[cfg(test)]
pub(crate) fn register(reg: &mut ToolRegistry) {
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
    let client = state.stores.client(mount_id).await?;
    if !client.is_file(path).await? {
        return Err(ToolError::not_found(format!("not a file: {path}")));
    }
    let offset = match marker {
        Some(raw) => decode_marker(raw, mount_id, ListKind::Tables, path)?,
        None => 0,
    };
    let store = client
        .trash
        .as_ref()
        .ok_or_else(|| ToolError::internal("volume has no relational meta store attached"))?;
    let set_id = store
        .current_artifact_set(path)
        .await?
        .ok_or_else(|| ToolError::not_found(NO_ARTIFACTS))?;
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
    if !client.is_file(path).await? {
        return Err(ToolError::not_found(format!("not a file: {path}")));
    }
    let format = TableFormat::parse(format)?;
    let store = client
        .trash
        .as_ref()
        .ok_or_else(|| ToolError::internal("volume has no relational meta store attached"))?;
    let set_id = store
        .current_artifact_set(path)
        .await?
        .ok_or_else(|| ToolError::not_found(NO_ARTIFACTS))?;
    let not_found = || ToolError::not_found(format!("Not found: {id}"));
    let seq = table_seq(id).ok_or_else(not_found)?;
    let raw = store.artifact_table_cells(&set_id, seq).await?.ok_or_else(not_found)?;
    let cells: Vec<Vec<String>> = serde_json::from_str(&raw)
        .map_err(|e| ToolError::internal(format!("stored table cells: {e}")))?;
    Ok(json!({"id": id, "format": format.as_str(), "content": format.render(&cells)}))
}

/// The `n` of a canonical `table-<n>` id, `n` at least 1 with no sign or
/// leading zero, so `table-01` is not an alias of `table-1`.
fn table_seq(id: &str) -> Option<i64> {
    let digits = id.strip_prefix("table-")?;
    let n: i64 = digits.parse().ok()?;
    (n >= 1 && n.to_string() == digits).then_some(n)
}

#[cfg(test)]
mod e2e {
    //! SPEC-0019 acceptance tests, driven through the real `McpServer` tool
    //! methods, the same dispatch path `app.rs` routes.

    use crate::docs::DocService;
    use crate::docs::service::StubDocService;
    use crate::mcp::server::{
        DocumentizeArgs, ExtractTextArgs, GetTableArgs, ListTablesArgs, McpServer, WriteBytesArgs,
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
        McpServer::new(Arc::new(state), ALICE.to_string())
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
}
