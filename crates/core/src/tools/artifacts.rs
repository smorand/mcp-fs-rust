//! Document artifacts (SPEC-0019): the tables a conversion kept, listed per
//! document.
//!
//! Typed functions only: the `#[tool]` method in `mcp::server` authorizes and
//! normalizes the path, then calls straight through to [`list_tables`], so
//! every door shares this one implementation (DEC-001).

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

#[cfg(test)]
mod e2e {
    //! SPEC-0019 acceptance tests, driven through the real `McpServer` tool
    //! methods, the same dispatch path `app.rs` routes.

    use crate::mcp::server::{ExtractTextArgs, ListTablesArgs, McpServer};
    use crate::tools::admin::test_support::Fixture;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::{CallToolResult, ContentBlock};
    use serde_json::{Value, json};

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
}
