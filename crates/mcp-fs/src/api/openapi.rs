//! The public OpenAPI document (`/api/swagger.json`) and Swagger UI (`/api/docs`)
//! for the `/api/fs` data plane. Port of the C# `Api/OpenApiToolDocs.cs` plus the
//! `AddOpenApi("swagger")` document transformer in `Program.BuildApp`.
//!
//! Single source of truth: not one summary or parameter description is written on
//! a REST endpoint. Every one of them is copied at request time from the MCP tool
//! schemas held by [`crate::mcp::ToolRegistry`], matching route
//! `/api/fs/{mount_id}/{sub}` to tool `fs.{sub_with_underscores}` (overrides:
//! `list` to `fs.list_dir`, `roots` to `fs.list_allowed_roots`). Consequences,
//! all deliberate and inherited from the C#:
//!
//! * documenting a tool parameter documents the REST endpoint too;
//! * the Swagger page is an audit of how well the tools are described for the
//!   LLM, a blank description there means a blank description in the tool schema;
//! * REST parameter names MUST equal the tool parameter names, which is why
//!   `/read-bytes` takes `offset_bytes` and `length_bytes`;
//! * the bytes plane routes have no tool, so they carry the small explicit
//!   summaries in [`REST_ONLY`] (the C# `RestOnly` map).
//!
//! The route and schema shapes themselves live in the tables below, because the
//! REST surface is not the tool surface: `/api/fs/{mount_id}/mkdir` takes only
//! `path` while `fs.mkdir` also takes `parents` and `exist_ok`. A test asserts the
//! table covers exactly [`super::dataplane::REST_ROUTES`], so a route can never
//! ship undocumented.
//!
//! Both endpoints are public: the C# identity middleware guards only the MCP
//! prefix, and an unauthenticated client still needs to read the docs to learn how
//! to authenticate. The Swagger UI assets are served from the copy embedded in
//! `utoipa-swagger-ui`, so the page works with no network access.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// Where the Swagger UI is mounted. Also the `<base href>` injected into the page.
const DOCS_PREFIX: &str = "/api/docs/";

/// The document router: the spec and the interactive page, both unauthenticated.
pub fn openapi_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/swagger.json", get(swagger_json))
        // Both spellings answer, so a trailing slash is never a 404.
        .route("/api/docs", get(docs_index))
        .route("/api/docs/", get(docs_index))
        .route("/api/docs/{*asset}", get(docs_asset))
        .with_state(state)
}

async fn swagger_json(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(document(&state))
}

// ───────────────────────────────────────────────────────────── the document ───

/// Build the OpenAPI 3.0.1 document for the current server.
///
/// Git paths appear only when the git subsystem is enabled, mirroring the C#
/// where those endpoints are simply not registered otherwise.
pub fn document(state: &AppState) -> Value {
    let catalog = catalog(state);

    let mut paths = Map::new();
    // The health probe is part of the surface and documented, without a tool.
    paths.insert("/health".into(), json!({"get": bare_operation(&[])}));
    for op in OPERATIONS {
        paths.insert(op.path.into(), json!({op.method.to_lowercase(): operation(op, &catalog)}));
    }
    if state.config.git.enabled {
        for (path, item) in git_paths() {
            paths.insert(path, item);
        }
    }

    let mut schemas = Map::new();
    for schema in SCHEMAS {
        schemas.insert(schema.name.into(), body_schema(schema, &catalog));
    }

    json!({
        "openapi": "3.0.1",
        "info": {
            "title": "mcp-fs REST API",
            "description": "The /api/fs data plane: bytes plane (upload/download/zip) plus \
                            full parity with the fs.* MCP tools. Auth: Bearer JWT in the \
                            Authorization or X-Forwarded-Authorization header.",
            "version": crate::app::VERSION,
        },
        "paths": paths,
        "components": {
            "schemas": schemas,
            "securitySchemes": {
                "Bearer": {
                    "type": "http",
                    "description": "RS256 JWT. Sent as 'Authorization: Bearer <token>' \
                                    (or X-Forwarded-Authorization when behind a gateway).",
                    "scheme": "bearer",
                    "bearerFormat": "JWT",
                }
            },
        },
        "tags": [{"name": "mcp-fs"}],
    })
}

/// Documentation of one MCP tool: the summary plus every parameter description.
#[derive(Default)]
struct ToolDoc {
    summary: String,
    params: HashMap<String, String>,
}

/// Reflect the live tool registry into `tool name -> ToolDoc`.
///
/// An empty registry is not an error: every description simply stays absent,
/// exactly as the C# leaves an operation blank when a tool has no `[Description]`.
fn catalog(state: &AppState) -> HashMap<String, ToolDoc> {
    let mut out = HashMap::new();
    let payload = state.registry.list_payload();
    let Some(tools) = payload.get("tools").and_then(Value::as_array) else {
        return out;
    };
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else { continue };
        let summary =
            tool.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut params = HashMap::new();
        if let Some(props) =
            tool.get("inputSchema").and_then(|s| s.get("properties")).and_then(Value::as_object)
        {
            for (param, schema) in props {
                if let Some(desc) = schema.get("description").and_then(Value::as_str)
                    && !desc.is_empty()
                {
                    params.insert(param.clone(), desc.to_string());
                }
            }
        }
        out.insert(name.to_string(), ToolDoc { summary, params });
    }
    out
}

/// An operation with no documentation to inherit (the health probe, git routes).
fn bare_operation(params: &[Value]) -> Value {
    let mut op = Map::new();
    op.insert("tags".into(), json!(["mcp-fs"]));
    if !params.is_empty() {
        op.insert("parameters".into(), json!(params));
    }
    op.insert("responses".into(), json!({"200": {"description": "OK"}}));
    Value::Object(op)
}

/// Render one documented operation, inheriting text from the tool catalog.
fn operation(op: &Op, catalog: &HashMap<String, ToolDoc>) -> Value {
    let tool = catalog.get(op.tool);
    let empty = ToolDoc::default();
    let doc = tool.unwrap_or(&empty);

    // Tool text wins; a bytes plane route falls back to its explicit summary.
    let (summary, description) = if tool.is_some() && !doc.summary.is_empty() {
        (doc.summary.clone(), format!("MCP tool: {}. {}", op.tool, doc.summary))
    } else {
        let rest = REST_ONLY.iter().find(|(sub, _)| *sub == op.sub).map(|(_, s)| *s);
        match rest {
            Some(s) => (s.to_string(), s.to_string()),
            None => (String::new(), String::new()),
        }
    };

    let describe = |name: &str| -> Option<String> {
        doc.params.get(name).cloned().or_else(|| {
            REST_ONLY_PARAMS
                .iter()
                .find(|(sub, param, _)| *sub == op.sub && *param == name)
                .map(|(_, _, desc)| desc.to_string())
        })
    };

    let mut parameters: Vec<Value> = Vec::new();
    if op.path.contains("{mount_id}") {
        parameters.push(parameter(
            "mount_id",
            "path",
            true,
            "string",
            "",
            Def::Absent,
            describe("mount_id"),
        ));
    }
    for p in op.params {
        parameters.push(parameter(
            p.name,
            "query",
            p.required,
            p.ty,
            p.format,
            p.default,
            describe(p.name),
        ));
    }

    let mut out = Map::new();
    out.insert("tags".into(), json!(["mcp-fs"]));
    if !summary.is_empty() {
        out.insert("summary".into(), json!(summary));
        out.insert("description".into(), json!(description));
    }
    if !parameters.is_empty() {
        out.insert("parameters".into(), Value::Array(parameters));
    }
    if !op.body.is_empty() {
        out.insert(
            "requestBody".into(),
            json!({
                "content": {"application/json": {
                    "schema": {"$ref": format!("#/components/schemas/{}", op.body)}
                }},
                "required": true,
            }),
        );
    }
    out.insert("responses".into(), json!({"200": {"description": "OK"}}));
    Value::Object(out)
}

fn parameter(
    name: &str,
    location: &str,
    required: bool,
    ty: &str,
    format: &str,
    default: Def,
    description: Option<String>,
) -> Value {
    let mut schema = Map::new();
    schema.insert("type".into(), json!(ty));
    if !format.is_empty() {
        schema.insert("format".into(), json!(format));
    }
    if let Some(d) = default.value() {
        schema.insert("default".into(), d);
    }

    let mut out = Map::new();
    out.insert("name".into(), json!(name));
    out.insert("in".into(), json!(location));
    if let Some(d) = description {
        out.insert("description".into(), json!(d));
    }
    if required {
        out.insert("required".into(), json!(true));
    }
    out.insert("schema".into(), Value::Object(schema));
    Value::Object(out)
}

/// Render one request body schema, inheriting field descriptions from the tool.
fn body_schema(schema: &Schema, catalog: &HashMap<String, ToolDoc>) -> Value {
    let empty = ToolDoc::default();
    let doc = catalog.get(schema.tool).unwrap_or(&empty);

    let mut props = Map::new();
    for p in schema.props {
        let mut field = Map::new();
        field.insert("type".into(), json!(p.ty));
        if !p.items.is_empty() {
            let items = if let Some(name) = p.items.strip_prefix('$') {
                json!({"$ref": format!("#/components/schemas/{name}")})
            } else {
                json!({"type": p.items})
            };
            field.insert("items".into(), items);
        }
        if let Some(desc) = doc.params.get(p.name) {
            field.insert("description".into(), json!(desc));
        }
        if !p.format.is_empty() {
            field.insert("format".into(), json!(p.format));
        }
        if let Some(d) = p.default.value() {
            field.insert("default".into(), d);
        }
        if p.nullable {
            field.insert("nullable".into(), json!(true));
        }
        props.insert(p.name.into(), Value::Object(field));
    }

    let mut out = Map::new();
    if !schema.required.is_empty() {
        out.insert("required".into(), json!(schema.required));
    }
    out.insert("type".into(), json!("object"));
    out.insert("properties".into(), Value::Object(props));
    Value::Object(out)
}

/// The git HTTP routes, documented without descriptions like the C# does (they
/// are wire protocol endpoints for the git CLI, not for a human or an LLM).
fn git_paths() -> Vec<(String, Value)> {
    let mount = json!({"name": "mount_id", "in": "path", "required": true, "schema": {"type": "string"}});
    let service = json!({"name": "service", "in": "query", "schema": {"type": "string"}});
    vec![
        (
            "/git/{mount_id}/info/refs".into(),
            json!({"get": bare_operation(&[mount.clone(), service])}),
        ),
        (
            "/git/{mount_id}/git-upload-pack".into(),
            json!({"post": bare_operation(std::slice::from_ref(&mount))}),
        ),
        (
            "/git/{mount_id}/git-receive-pack".into(),
            json!({"post": bare_operation(std::slice::from_ref(&mount))}),
        ),
    ]
}

// ──────────────────────────────────────────────────────────── the route table ──

/// A rendered default value. `Absent` omits the key, `Null` renders `null` (an
/// optional parameter with no value), both of which the C# generator produces.
#[derive(Clone, Copy)]
enum Def {
    Absent,
    Null,
    Bool(bool),
    Int(i64),
    Str(&'static str),
}

impl Def {
    fn value(self) -> Option<Value> {
        match self {
            Def::Absent => None,
            Def::Null => Some(Value::Null),
            Def::Bool(b) => Some(json!(b)),
            Def::Int(i) => Some(json!(i)),
            Def::Str(s) => Some(json!(s)),
        }
    }
}

/// One query parameter of a REST route.
struct Param {
    name: &'static str,
    required: bool,
    ty: &'static str,
    /// OpenAPI numeric format, or "" for none.
    format: &'static str,
    default: Def,
}

/// One documented REST operation.
struct Op {
    method: &'static str,
    /// Last path segment, the key shared with [`super::dataplane::REST_ROUTES`].
    sub: &'static str,
    path: &'static str,
    /// MCP tool whose text this operation inherits, "" for the bytes plane.
    tool: &'static str,
    params: &'static [Param],
    /// Request body schema name, "" for a GET.
    body: &'static str,
}

/// Summaries for the routes that have no MCP tool (the C# `RestOnly` map).
const REST_ONLY: &[(&str, &str)] = &[
    ("upload", "Upload one or more files (multipart form) into a directory."),
    ("download", "Download a single file's raw bytes as an attachment."),
    ("download-zip", "Download a directory subtree as a zip archive."),
];

/// Parameter descriptions for those same tool free routes.
const REST_ONLY_PARAMS: &[(&str, &str, &str)] = &[
    ("upload", "mount_id", "Project/volume id the operation targets."),
    ("download", "mount_id", "Project/volume id the operation targets."),
    ("download", "path", "Absolute POSIX path of the file to download."),
    ("download-zip", "mount_id", "Project/volume id the operation targets."),
    (
        "download-zip",
        "path",
        "Absolute POSIX directory to archive (defaults to the volume root).",
    ),
];

const NO_PARAMS: &[Param] = &[];

/// `path` as a required query parameter, the shape most read routes use.
const PATH_REQ: &[Param] =
    &[Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent }];

/// Every documented operation, in the C# registration order.
const OPERATIONS: &[Op] = &[
    Op {
        method: "GET",
        sub: "roots",
        path: "/api/fs/roots",
        tool: "fs.list_allowed_roots",
        params: NO_PARAMS,
        body: "",
    },
    Op {
        method: "GET",
        sub: "list",
        path: "/api/fs/{mount_id}/list",
        tool: "fs.list_dir",
        params: &[Param {
            name: "path",
            required: false,
            ty: "string",
            format: "",
            default: Def::Absent,
        }],
        body: "",
    },
    Op {
        method: "POST",
        sub: "mkdir",
        path: "/api/fs/{mount_id}/mkdir",
        tool: "fs.mkdir",
        params: NO_PARAMS,
        body: "MkdirBody",
    },
    Op {
        method: "POST",
        sub: "delete",
        path: "/api/fs/{mount_id}/delete",
        tool: "fs.delete",
        params: NO_PARAMS,
        body: "DeleteBody",
    },
    Op {
        method: "POST",
        sub: "move",
        path: "/api/fs/{mount_id}/move",
        tool: "fs.move",
        params: NO_PARAMS,
        body: "MoveBody",
    },
    Op {
        method: "POST",
        sub: "upload",
        path: "/api/fs/{mount_id}/upload",
        tool: "",
        params: NO_PARAMS,
        body: "",
    },
    Op {
        method: "GET",
        sub: "download",
        path: "/api/fs/{mount_id}/download",
        tool: "",
        params: PATH_REQ,
        body: "",
    },
    Op {
        method: "GET",
        sub: "download-zip",
        path: "/api/fs/{mount_id}/download-zip",
        tool: "",
        params: &[Param {
            name: "path",
            required: false,
            ty: "string",
            format: "",
            default: Def::Absent,
        }],
        body: "",
    },
    Op {
        method: "GET",
        sub: "read",
        path: "/api/fs/{mount_id}/read",
        tool: "fs.read",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "offset_lines",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(0),
            },
            Param {
                name: "limit_lines",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(2000),
            },
            Param {
                name: "line_numbered",
                required: false,
                ty: "boolean",
                format: "",
                default: Def::Bool(true),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "read-bytes",
        path: "/api/fs/{mount_id}/read-bytes",
        tool: "fs.read_bytes",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "offset_bytes",
                required: false,
                ty: "integer",
                format: "int64",
                default: Def::Int(0),
            },
            Param {
                name: "length_bytes",
                required: false,
                ty: "integer",
                format: "int64",
                default: Def::Int(65536),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "stat",
        path: "/api/fs/{mount_id}/stat",
        tool: "fs.stat",
        params: PATH_REQ,
        body: "",
    },
    Op {
        method: "GET",
        sub: "exists",
        path: "/api/fs/{mount_id}/exists",
        tool: "fs.exists",
        params: PATH_REQ,
        body: "",
    },
    Op {
        method: "GET",
        sub: "hash",
        path: "/api/fs/{mount_id}/hash",
        tool: "fs.hash",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "algo",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("sha256"),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "count-lines",
        path: "/api/fs/{mount_id}/count-lines",
        tool: "fs.count_lines",
        params: PATH_REQ,
        body: "",
    },
    Op {
        method: "GET",
        sub: "glob",
        path: "/api/fs/{mount_id}/glob",
        tool: "fs.glob",
        params: &[
            Param { name: "pattern", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "root",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("/"),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "grep",
        path: "/api/fs/{mount_id}/grep",
        tool: "fs.grep",
        params: &[
            Param { name: "pattern", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "root",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("/"),
            },
            Param {
                name: "include_glob",
                required: false,
                ty: "string",
                format: "",
                default: Def::Null,
            },
            Param {
                name: "exclude_glob",
                required: false,
                ty: "string",
                format: "",
                default: Def::Null,
            },
            Param {
                name: "regex",
                required: false,
                ty: "boolean",
                format: "",
                default: Def::Bool(true),
            },
            Param {
                name: "case_sensitive",
                required: false,
                ty: "boolean",
                format: "",
                default: Def::Bool(true),
            },
            Param {
                name: "output_mode",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("content"),
            },
            Param {
                name: "context_lines",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(0),
            },
            Param {
                name: "max_matches",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(100),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "tree",
        path: "/api/fs/{mount_id}/tree",
        tool: "fs.tree",
        params: &[
            Param {
                name: "path",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("/"),
            },
            Param {
                name: "max_depth",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(3),
            },
            Param {
                name: "with_sizes",
                required: false,
                ty: "boolean",
                format: "",
                default: Def::Bool(false),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "read-lines",
        path: "/api/fs/{mount_id}/read-lines",
        tool: "fs.read_lines",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "start_line",
                required: true,
                ty: "integer",
                format: "int32",
                default: Def::Absent,
            },
            Param {
                name: "end_line",
                required: true,
                ty: "integer",
                format: "int32",
                default: Def::Absent,
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "read-section",
        path: "/api/fs/{mount_id}/read-section",
        tool: "fs.read_section",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "anchor_line",
                required: true,
                ty: "integer",
                format: "int32",
                default: Def::Absent,
            },
            Param {
                name: "max_lines",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(200),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "head",
        path: "/api/fs/{mount_id}/head",
        tool: "fs.head",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "lines",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(20),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "tail",
        path: "/api/fs/{mount_id}/tail",
        tool: "fs.tail",
        params: &[
            Param { name: "path", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "lines",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(20),
            },
        ],
        body: "",
    },
    Op {
        method: "POST",
        sub: "read-many",
        path: "/api/fs/{mount_id}/read-many",
        tool: "fs.read_many",
        params: NO_PARAMS,
        body: "ReadManyBody",
    },
    Op {
        method: "POST",
        sub: "copy",
        path: "/api/fs/{mount_id}/copy",
        tool: "fs.copy",
        params: NO_PARAMS,
        body: "CopyBody",
    },
    Op {
        method: "POST",
        sub: "write",
        path: "/api/fs/{mount_id}/write",
        tool: "fs.write",
        params: NO_PARAMS,
        body: "WriteBody",
    },
    Op {
        method: "POST",
        sub: "append",
        path: "/api/fs/{mount_id}/append",
        tool: "fs.append",
        params: NO_PARAMS,
        body: "AppendBody",
    },
    Op {
        method: "POST",
        sub: "create-empty",
        path: "/api/fs/{mount_id}/create-empty",
        tool: "fs.create_empty",
        params: NO_PARAMS,
        body: "CreateEmptyBody",
    },
    Op {
        method: "POST",
        sub: "edit",
        path: "/api/fs/{mount_id}/edit",
        tool: "fs.edit",
        params: NO_PARAMS,
        body: "EditBody",
    },
    Op {
        method: "POST",
        sub: "multi-edit",
        path: "/api/fs/{mount_id}/multi-edit",
        tool: "fs.multi_edit",
        params: NO_PARAMS,
        body: "MultiEditBody",
    },
    Op {
        method: "POST",
        sub: "search-replace",
        path: "/api/fs/{mount_id}/search-replace",
        tool: "fs.search_replace",
        params: NO_PARAMS,
        body: "SearchReplaceBody",
    },
    Op {
        method: "POST",
        sub: "insert-at-line",
        path: "/api/fs/{mount_id}/insert-at-line",
        tool: "fs.insert_at_line",
        params: NO_PARAMS,
        body: "InsertAtLineBody",
    },
    Op {
        method: "POST",
        sub: "apply-patch",
        path: "/api/fs/{mount_id}/apply-patch",
        tool: "fs.apply_patch",
        params: NO_PARAMS,
        body: "ApplyPatchBody",
    },
    Op {
        method: "POST",
        sub: "extract-text",
        path: "/api/fs/{mount_id}/extract-text",
        tool: "fs.extract_text",
        params: NO_PARAMS,
        body: "ExtractBody",
    },
    Op {
        method: "POST",
        sub: "write-docx",
        path: "/api/fs/{mount_id}/write-docx",
        tool: "fs.write_docx",
        params: NO_PARAMS,
        body: "WriteDocxBody",
    },
    Op {
        method: "GET",
        sub: "find-definition",
        path: "/api/fs/{mount_id}/find-definition",
        tool: "fs.find_definition",
        params: &[
            Param { name: "name", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "root",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("/"),
            },
            Param { name: "kind", required: false, ty: "string", format: "", default: Def::Null },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "find-references",
        path: "/api/fs/{mount_id}/find-references",
        tool: "fs.find_references",
        params: &[
            Param { name: "name", required: true, ty: "string", format: "", default: Def::Absent },
            Param {
                name: "root",
                required: false,
                ty: "string",
                format: "",
                default: Def::Str("/"),
            },
        ],
        body: "",
    },
    Op {
        method: "GET",
        sub: "audit-log",
        path: "/api/fs/{mount_id}/audit-log",
        tool: "fs.audit_log",
        params: &[
            Param {
                name: "since",
                required: false,
                ty: "number",
                format: "double",
                default: Def::Absent,
            },
            Param {
                name: "limit",
                required: false,
                ty: "integer",
                format: "int32",
                default: Def::Int(20),
            },
        ],
        body: "",
    },
];

// ─────────────────────────────────────────────────────────── request bodies ───

/// One field of a request body schema.
struct Prop {
    name: &'static str,
    ty: &'static str,
    format: &'static str,
    default: Def,
    /// Array item type: "" for a scalar, "string", or "$SchemaName" for a `$ref`.
    items: &'static str,
    nullable: bool,
}

/// A request body schema and the tool it inherits field descriptions from.
struct Schema {
    name: &'static str,
    tool: &'static str,
    required: &'static [&'static str],
    props: &'static [Prop],
}

/// Shorthand for a plain field with no format, default, items or nullability.
const fn plain(name: &'static str, ty: &'static str) -> Prop {
    Prop { name, ty, format: "", default: Def::Absent, items: "", nullable: false }
}

const fn flag(name: &'static str, default: bool) -> Prop {
    Prop { name, ty: "boolean", format: "", default: Def::Bool(default), items: "", nullable: false }
}

const fn counter(name: &'static str, default: i64) -> Prop {
    Prop {
        name,
        ty: "integer",
        format: "int32",
        default: Def::Int(default),
        items: "",
        nullable: false,
    }
}

const SCHEMAS: &[Schema] = &[
    Schema {
        name: "MkdirBody",
        tool: "fs.mkdir",
        required: &["path"],
        props: &[plain("path", "string")],
    },
    Schema {
        name: "DeleteBody",
        tool: "fs.delete",
        required: &["path"],
        props: &[plain("path", "string")],
    },
    Schema {
        name: "MoveBody",
        tool: "fs.move",
        required: &["source", "destination"],
        props: &[plain("source", "string"), plain("destination", "string")],
    },
    Schema {
        name: "CopyBody",
        tool: "fs.copy",
        required: &["source", "destination"],
        props: &[
            plain("source", "string"),
            plain("destination", "string"),
            flag("overwrite", false),
            flag("recursive", false),
        ],
    },
    Schema {
        name: "WriteBody",
        tool: "fs.write",
        required: &["path", "content"],
        props: &[
            plain("path", "string"),
            plain("content", "string"),
            flag("overwrite", false),
            flag("create_parents", true),
        ],
    },
    Schema {
        name: "AppendBody",
        tool: "fs.append",
        required: &["path", "content"],
        props: &[plain("path", "string"), plain("content", "string"), flag("create", false)],
    },
    Schema {
        name: "CreateEmptyBody",
        tool: "fs.create_empty",
        required: &["path"],
        props: &[plain("path", "string"), flag("exist_ok", false)],
    },
    Schema {
        name: "EditBody",
        tool: "fs.edit",
        required: &["path", "old_string", "new_string"],
        props: &[
            plain("path", "string"),
            plain("old_string", "string"),
            plain("new_string", "string"),
            flag("replace_all", false),
            flag("dry_run", false),
        ],
    },
    Schema {
        name: "EditSpec",
        // No tool: the C# EditSpec record carries no [Description], and inheriting
        // fs.edit's text here would describe the wrong thing (one edit of many).
        tool: "",
        required: &[],
        props: &[
            plain("old_string", "string"),
            plain("new_string", "string"),
            plain("replace_all", "boolean"),
        ],
    },
    Schema {
        name: "MultiEditBody",
        tool: "fs.multi_edit",
        required: &["path", "edits"],
        props: &[
            plain("path", "string"),
            Prop {
                name: "edits",
                ty: "array",
                format: "",
                default: Def::Absent,
                items: "$EditSpec",
                nullable: false,
            },
            flag("dry_run", false),
        ],
    },
    Schema {
        name: "SearchReplaceBody",
        tool: "fs.search_replace",
        required: &["path", "search_block", "replace_block"],
        props: &[
            plain("path", "string"),
            plain("search_block", "string"),
            plain("replace_block", "string"),
            flag("fuzzy", false),
        ],
    },
    Schema {
        name: "InsertAtLineBody",
        tool: "fs.insert_at_line",
        required: &["path", "line", "content"],
        props: &[
            plain("path", "string"),
            Prop {
                name: "line",
                ty: "integer",
                format: "int32",
                default: Def::Absent,
                items: "",
                nullable: false,
            },
            plain("content", "string"),
        ],
    },
    Schema {
        name: "ApplyPatchBody",
        tool: "fs.apply_patch",
        required: &["patch_text"],
        props: &[plain("patch_text", "string")],
    },
    Schema {
        name: "ReadManyBody",
        tool: "fs.read_many",
        required: &["paths"],
        props: &[
            Prop {
                name: "paths",
                ty: "array",
                format: "",
                default: Def::Absent,
                items: "string",
                nullable: false,
            },
            counter("per_file_cap_lines", 500),
        ],
    },
    Schema {
        name: "ExtractBody",
        tool: "fs.extract_text",
        required: &["path"],
        props: &[
            plain("path", "string"),
            counter("max_chars", 200_000),
            counter("preview_chars", 4_000),
            flag("ocr", true),
            flag("refresh", false),
        ],
    },
    Schema {
        name: "WriteDocxBody",
        tool: "fs.write_docx",
        required: &["path", "markdown"],
        props: &[
            plain("path", "string"),
            plain("markdown", "string"),
            Prop {
                name: "title",
                ty: "string",
                format: "",
                default: Def::Null,
                items: "",
                nullable: true,
            },
            flag("overwrite", false),
        ],
    },
];

// ─────────────────────────────────────────────────────────────── swagger ui ───

/// Swagger UI configuration: one spec, ours. Built once, it never changes.
static UI_CONFIG: LazyLock<Arc<utoipa_swagger_ui::Config<'static>>> =
    LazyLock::new(|| Arc::new(utoipa_swagger_ui::Config::from("/api/swagger.json")));

/// The interactive page. A `<base href>` is injected because the vendored
/// index.html references its assets relatively, and `/api/docs` (no trailing
/// slash) would otherwise resolve them one directory too high.
async fn docs_index() -> Response {
    match asset("index.html") {
        Some((bytes, content_type)) => {
            let html = String::from_utf8_lossy(&bytes)
                .replace("<head>", &format!("<head>\n    <base href=\"{DOCS_PREFIX}\">"))
                .replace("<title>Swagger UI</title>", "<title>mcp-fs API</title>");
            ([(header::CONTENT_TYPE, content_type)], html).into_response()
        }
        None => (StatusCode::NOT_FOUND, "swagger ui assets are missing").into_response(),
    }
}

/// One Swagger UI asset (css, js, fonts) from the embedded distribution.
async fn docs_asset(Path(name): Path<String>) -> Response {
    match asset(&name) {
        Some((bytes, content_type)) => {
            ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Look up an embedded Swagger UI file, returning its bytes and content type.
fn asset(name: &str) -> Option<(Vec<u8>, String)> {
    match utoipa_swagger_ui::serve(name, UI_CONFIG.clone()) {
        Ok(Some(file)) => Some((file.bytes.to_vec(), file.content_type)),
        // A missing asset and a broken asset are both a 404 for the caller; the
        // page is a convenience, never a reason to fail a request.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::dataplane::REST_ROUTES;
    use crate::config::ServerConfig;
    use crate::mcp::ToolSchema;
    use crate::mcp::registry::{ToolRegistry, handler};
    use crate::safety::SafetyManager;
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    /// A state whose registry holds a couple of real tool schemas, so the
    /// inheritance path is exercised even before the tool families land.
    async fn state_with_tools(dir: &std::path::Path, with_tools: bool, git: bool) -> Arc<AppState> {
        let mut config = ServerConfig::default();
        config.infra.meta.dir = dir.join("volumes").display().to_string();
        config.infra.blob.dir = dir.join("blobs").display().to_string();
        config.infra.admin.path = dir.join("admin.db").display().to_string();
        config.git.enabled = git;
        let config = Arc::new(config);

        let admin = crate::storage::build_admin_store(&config).unwrap();
        admin.connect().await.unwrap();

        let mut registry = ToolRegistry::new();
        if with_tools {
            registry.add(
                ToolSchema::new("fs.read", "Read a text file with line-numbered, paged output.")
                    .req_str("mount_id", "Project/volume id the operation targets.")
                    .req_str("path", "Absolute POSIX path within the volume, e.g. /src/app.py.")
                    .opt_int("offset_lines", 0, "0-based line offset to start reading from.")
                    .opt_int("limit_lines", 2000, "Maximum number of lines to return.")
                    .opt_bool("line_numbered", true, "Prefix each line with its 1-based line number."),
                handler(|_c, _a| async move { Ok(json!({})) }),
            );
            registry.add(
                ToolSchema::new("fs.write", "Create or overwrite a file (no-clobber by default, atomic).")
                    .req_str("mount_id", "Project/volume id the operation targets.")
                    .req_str("path", "Absolute POSIX path within the volume, e.g. /src/app.py.")
                    .req_str("content", "Full text content to write to the file.")
                    .opt_bool("overwrite", false, "Allow overwriting an existing file (default no-clobber).")
                    .opt_bool("create_parents", true, "Create missing parent directories."),
                handler(|_c, _a| async move { Ok(json!({})) }),
            );
        }

        Arc::new(AppState {
            config: config.clone(),
            admin,
            stores: Arc::new(crate::storage::StoreManager::new(config.clone())),
            safety: Arc::new(SafetyManager::new(config.safety.clone())),
            identity: Arc::new(crate::identity::IdentityResolver::new(&config.auth)),
            registry: Arc::new(registry),
        })
    }

    async fn doc(with_tools: bool, git: bool) -> Value {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_tools(dir.path(), with_tools, git).await;
        document(&state)
    }

    #[tokio::test]
    async fn swagger_json_is_public_and_well_formed() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_tools(dir.path(), true, false).await;
        let response = openapi_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/swagger.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["openapi"], "3.0.1");
        assert_eq!(v["info"]["title"], "mcp-fs REST API");
        assert_eq!(v["info"]["version"], crate::app::VERSION);
        assert!(v["info"]["description"].as_str().unwrap().contains("/api/fs data plane"));
        assert_eq!(v["tags"][0]["name"], "mcp-fs");
    }

    #[tokio::test]
    async fn the_bearer_security_scheme_is_declared() {
        let v = doc(true, false).await;
        let bearer = &v["components"]["securitySchemes"]["Bearer"];
        assert_eq!(bearer["type"], "http");
        assert_eq!(bearer["scheme"], "bearer");
        assert_eq!(bearer["bearerFormat"], "JWT");
        assert!(bearer["description"].as_str().unwrap().contains("RS256"));
    }

    #[tokio::test]
    async fn every_route_of_the_data_plane_is_documented() {
        let v = doc(false, false).await;
        let paths = v["paths"].as_object().unwrap();
        for (method, sub) in REST_ROUTES {
            let op = OPERATIONS
                .iter()
                .find(|o| o.sub == *sub && o.method == *method)
                .unwrap_or_else(|| panic!("route {method} {sub} is not in the OpenAPI table"));
            let item = paths
                .get(op.path)
                .unwrap_or_else(|| panic!("path {} missing from the document", op.path));
            assert!(
                item.get(method.to_lowercase()).is_some(),
                "method {method} missing on {}",
                op.path
            );
        }
        assert_eq!(OPERATIONS.len(), REST_ROUTES.len(), "the two tables must stay in step");
    }

    #[tokio::test]
    async fn the_document_has_no_operation_outside_the_route_table() {
        let v = doc(false, false).await;
        for path in v["paths"].as_object().unwrap().keys() {
            if path == "/health" || path.starts_with("/git/") {
                continue;
            }
            assert!(
                OPERATIONS.iter().any(|o| o.path == path),
                "documented path {path} has no route"
            );
        }
    }

    #[tokio::test]
    async fn descriptions_are_inherited_from_the_tool_schemas() {
        let v = doc(true, false).await;
        let read = &v["paths"]["/api/fs/{mount_id}/read"]["get"];
        assert_eq!(read["summary"], "Read a text file with line-numbered, paged output.");
        assert_eq!(
            read["description"],
            "MCP tool: fs.read. Read a text file with line-numbered, paged output."
        );
        let params = read["parameters"].as_array().unwrap();
        assert_eq!(params[0]["name"], "mount_id");
        assert_eq!(params[0]["in"], "path");
        assert_eq!(params[0]["description"], "Project/volume id the operation targets.");
        assert_eq!(params[1]["name"], "path");
        assert_eq!(params[1]["required"], true);
        assert!(params[1]["description"].as_str().unwrap().contains("Absolute POSIX path"));
        assert_eq!(params[2]["name"], "offset_lines");
        assert_eq!(params[2]["schema"]["default"], 0);
        assert_eq!(params[2]["description"], "0-based line offset to start reading from.");
        assert_eq!(params[4]["schema"]["default"], true);
    }

    #[tokio::test]
    async fn body_field_descriptions_are_inherited_too() {
        let v = doc(true, false).await;
        let schema = &v["components"]["schemas"]["WriteBody"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["path", "content"]));
        assert_eq!(schema["properties"]["content"]["description"], "Full text content to write to the file.");
        assert_eq!(schema["properties"]["create_parents"]["default"], true);
        assert_eq!(
            v["paths"]["/api/fs/{mount_id}/write"]["post"]["requestBody"]["content"]
                ["application/json"]["schema"]["$ref"],
            "#/components/schemas/WriteBody"
        );
    }

    /// The audit value of the page: an undocumented tool parameter shows up blank
    /// rather than getting invented text here.
    #[tokio::test]
    async fn an_empty_registry_leaves_tool_descriptions_blank() {
        let v = doc(false, false).await;
        let read = &v["paths"]["/api/fs/{mount_id}/read"]["get"];
        assert!(read.get("summary").is_none());
        assert!(read["parameters"][1].get("description").is_none());
        // Structure survives regardless.
        assert_eq!(read["parameters"][1]["name"], "path");
        assert_eq!(read["responses"]["200"]["description"], "OK");
    }

    /// The bytes plane has no tool, so its text comes from the explicit table.
    #[tokio::test]
    async fn tool_free_routes_keep_their_own_summaries() {
        let v = doc(false, false).await;
        let download = &v["paths"]["/api/fs/{mount_id}/download"]["get"];
        assert_eq!(download["summary"], "Download a single file's raw bytes as an attachment.");
        assert_eq!(download["parameters"][1]["description"], "Absolute POSIX path of the file to download.");
        let zip = &v["paths"]["/api/fs/{mount_id}/download-zip"]["get"];
        assert_eq!(zip["summary"], "Download a directory subtree as a zip archive.");
        let upload = &v["paths"]["/api/fs/{mount_id}/upload"]["post"];
        assert_eq!(upload["summary"], "Upload one or more files (multipart form) into a directory.");
        assert!(upload.get("requestBody").is_none(), "multipart is not a json body");
    }

    #[tokio::test]
    async fn read_bytes_documents_the_tool_parameter_names() {
        let v = doc(false, false).await;
        let names: Vec<&str> = v["paths"]["/api/fs/{mount_id}/read-bytes"]["get"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["mount_id", "path", "offset_bytes", "length_bytes"]);
    }

    #[tokio::test]
    async fn nullable_and_absent_defaults_render_as_the_csharp_does() {
        let v = doc(false, false).await;
        let grep = &v["paths"]["/api/fs/{mount_id}/grep"]["get"]["parameters"];
        let include = grep.as_array().unwrap().iter().find(|p| p["name"] == "include_glob").unwrap();
        assert!(include["schema"]["default"].is_null());
        assert!(include["schema"].as_object().unwrap().contains_key("default"));

        let since = v["paths"]["/api/fs/{mount_id}/audit-log"]["get"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "since")
            .unwrap()
            .clone();
        assert_eq!(since["schema"]["type"], "number");
        assert_eq!(since["schema"]["format"], "double");
        assert!(!since["schema"].as_object().unwrap().contains_key("default"));

        let title = &v["components"]["schemas"]["WriteDocxBody"]["properties"]["title"];
        assert_eq!(title["nullable"], true);
        assert!(title["default"].is_null());
    }

    #[tokio::test]
    async fn array_bodies_declare_their_item_schema() {
        let v = doc(false, false).await;
        let paths = &v["components"]["schemas"]["ReadManyBody"]["properties"]["paths"];
        assert_eq!(paths["type"], "array");
        assert_eq!(paths["items"]["type"], "string");
        let edits = &v["components"]["schemas"]["MultiEditBody"]["properties"]["edits"];
        assert_eq!(edits["items"]["$ref"], "#/components/schemas/EditSpec");
        let spec = &v["components"]["schemas"]["EditSpec"];
        assert!(spec.get("required").is_none());
        assert_eq!(spec["properties"]["replace_all"]["type"], "boolean");
    }

    #[tokio::test]
    async fn every_body_reference_resolves() {
        let v = doc(false, false).await;
        let schemas = v["components"]["schemas"].as_object().unwrap();
        for op in OPERATIONS.iter().filter(|o| !o.body.is_empty()) {
            assert!(schemas.contains_key(op.body), "schema {} is missing", op.body);
        }
        assert_eq!(schemas.len(), SCHEMAS.len());
    }

    #[tokio::test]
    async fn health_is_documented_and_git_follows_the_config() {
        let without = doc(false, false).await;
        assert!(without["paths"]["/health"]["get"]["responses"]["200"].is_object());
        assert!(without["paths"].get("/git/{mount_id}/info/refs").is_none());

        let with = doc(false, true).await;
        let refs = &with["paths"]["/git/{mount_id}/info/refs"]["get"];
        assert_eq!(refs["parameters"][0]["name"], "mount_id");
        assert_eq!(refs["parameters"][1]["name"], "service");
        assert!(with["paths"]["/git/{mount_id}/git-receive-pack"]["post"].is_object());
    }

    #[tokio::test]
    async fn roots_is_documented_even_though_it_takes_no_mount_id() {
        let v = doc(false, false).await;
        let roots = &v["paths"]["/api/fs/roots"]["get"];
        assert!(roots.is_object());
        assert!(roots.get("parameters").is_none());
    }

    // ── swagger ui ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn docs_serves_html_without_auth() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_tools(dir.path(), false, false).await;
        let response = openapi_router(state)
            .oneshot(Request::builder().uri("/api/docs").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"),
            "content type was {:?}",
            response.headers()[header::CONTENT_TYPE]
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("swagger-ui"), "not the swagger page: {html}");
        assert!(html.contains("<base href=\"/api/docs/\">"), "missing base href");
        assert!(html.contains("<title>mcp-fs API</title>"));
    }

    #[tokio::test]
    async fn docs_assets_are_served_from_the_embedded_distribution() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_tools(dir.path(), false, false).await;
        let app = openapi_router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/docs/swagger-initializer.js")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let js = String::from_utf8_lossy(&body);
        assert!(js.contains("/api/swagger.json"), "the ui must point at our spec: {js}");

        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/api/docs/nope.txt")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_trailing_slash_spelling_also_serves_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_tools(dir.path(), false, false).await;
        let response = openapi_router(state)
            .oneshot(Request::builder().uri("/api/docs/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

