//! Declarative schema, rendered to DDL per dialect.
//!
//! A table is declared once instead of as one hand written `CREATE TABLE` string
//! per engine, so a column cannot be added to the SQLite schema and forgotten in
//! the others. Rendering is idempotent: applying a `SchemaSet` twice is a no op,
//! which is what lets every store call `migrate` on open.

use super::dialect::{ColumnType, Dialect};

/// One column declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: &'static str,
    pub ty: ColumnType,
    pub not_null: bool,
    /// Raw SQL default, dialect neutral (for example `0`).
    pub default: Option<&'static str>,
}

impl Column {
    /// Nullable column.
    pub fn new(name: &'static str, ty: ColumnType) -> Self {
        Self { name, ty, not_null: false, default: None }
    }

    /// `NOT NULL` column.
    pub fn required(name: &'static str, ty: ColumnType) -> Self {
        Self { name, ty, not_null: true, default: None }
    }

    pub fn default(mut self, sql: &'static str) -> Self {
        self.default = Some(sql);
        self
    }
}

/// A foreign key with an optional cascade, used by the ACL registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKey {
    pub columns: Vec<&'static str>,
    pub references_table: &'static str,
    pub references_columns: Vec<&'static str>,
    pub on_delete_cascade: bool,
}

/// One table declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: &'static str,
    pub columns: Vec<Column>,
    pub primary_key: Vec<&'static str>,
    pub foreign_keys: Vec<ForeignKey>,
}

impl Table {
    pub fn new(name: &'static str, columns: Vec<Column>, primary_key: Vec<&'static str>) -> Self {
        Self { name, columns, primary_key, foreign_keys: Vec::new() }
    }

    pub fn foreign_key(mut self, fk: ForeignKey) -> Self {
        self.foreign_keys.push(fk);
        self
    }
}

/// A non unique secondary index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub name: &'static str,
    pub table: &'static str,
    pub columns: Vec<&'static str>,
}

/// Index kind, used by [`TypedIndex`] to render PostgreSQL-specific index types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexKind {
    /// Standard B-tree index (the default).
    Standard,
    /// GIN index (PostgreSQL only): used for tsvector full-text columns.
    Gin,
    /// IVFFlat index (PostgreSQL only): used for vector cosine similarity.
    IvfFlat,
}

/// An index with an optional PostgreSQL-specific access method.
///
/// On non-PostgreSQL dialects, `Gin` renders as a standard index and `IvfFlat`
/// is skipped entirely (sqlite-vec stores vectors in a vec0 virtual table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedIndex {
    pub name: &'static str,
    pub table: &'static str,
    pub columns: Vec<&'static str>,
    pub kind: IndexKind,
}

/// A virtual table declaration (SQLite only, for vec0 or FTS5).
///
/// These cannot be expressed as a column list, so they live outside the
/// regular `Table` declaration and are emitted after regular tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualTableDef {
    /// Table name.
    pub name: &'static str,
    /// Extension name, e.g. "vec0" or "fts5".
    pub using: &'static str,
    /// Column or parameter strings, e.g. ["embedding float[1536]"].
    pub columns: Vec<String>,
    /// When `Some(Dialect::Sqlite)`, only emit on that dialect and skip others.
    pub dialect: Option<super::dialect::Dialect>,
}

/// One column added to a table that a deployed database already has.
///
/// `CREATE TABLE IF NOT EXISTS` cannot widen an existing table, so a column added
/// after a release needs its own `ALTER TABLE`. Declaring it here keeps the DDL in
/// the schema layer and rendered per dialect, instead of a hand written statement
/// per engine in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMigration {
    pub table: &'static str,
    pub column: &'static str,
    pub ty: ColumnType,
    pub not_null: bool,
    /// Raw SQL literal, for example `'none'`. Required: an existing row needs a
    /// value for a `NOT NULL` column.
    pub default: &'static str,
}

/// A table dropped and recreated when a live deployment predates a shape
/// change no [`ColumnMigration`] can express (for example a primary key
/// change). Guarded so it fires only when the table exists but lacks
/// `guard_column`: never on a fresh install (no table at all) and never again
/// once the table already carries the new shape, so a restart never destroys
/// the rows it just rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRecreation {
    pub table: &'static str,
    pub guard_column: &'static str,
}

/// Every table and index one store needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaSet {
    pub tables: Vec<Table>,
    pub indexes: Vec<Index>,
    pub typed_indexes: Vec<TypedIndex>,
    pub virtual_tables: Vec<VirtualTableDef>,
    pub column_migrations: Vec<ColumnMigration>,
    pub table_recreations: Vec<TableRecreation>,
}

impl SchemaSet {
    pub fn new(tables: Vec<Table>, indexes: Vec<Index>) -> Self {
        Self {
            tables,
            indexes,
            typed_indexes: Vec::new(),
            virtual_tables: Vec::new(),
            column_migrations: Vec::new(),
            table_recreations: Vec::new(),
        }
    }

    /// Declare a column added to an already deployed table.
    pub fn column_migration(mut self, m: ColumnMigration) -> Self {
        self.column_migrations.push(m);
        self
    }

    /// Declare a table dropped and rebuilt by the following `CREATE TABLE` when
    /// its live shape lacks `guard_column` (DRIFT-005).
    #[must_use]
    pub fn recreate_if_missing_column(
        mut self,
        table: &'static str,
        guard_column: &'static str,
    ) -> Self {
        self.table_recreations.push(TableRecreation { table, guard_column });
        self
    }

    /// The `ALTER TABLE ... ADD COLUMN` statements, rendered for this dialect.
    ///
    /// Kept out of [`Self::render`] because they are not self guarding on every
    /// engine: SQLite has no `ADD COLUMN IF NOT EXISTS`, so its `migrate` probes
    /// `pragma_table_info` first. PostgreSQL and SQL Server render their own
    /// guard and can run the statement blind.
    pub fn render_column_migrations(&self, dialect: Dialect) -> Vec<String> {
        self.column_migrations
            .iter()
            .map(|m| super::dialect::render_column_migration(dialect, m))
            .collect()
    }

    /// The DDL statements to apply, in order. Each is separately idempotent.
    pub fn render(&self, dialect: Dialect) -> Vec<String> {
        let mut out = Vec::with_capacity(
            self.tables.len()
                + self.indexes.len()
                + self.typed_indexes.len()
                + self.virtual_tables.len(),
        );
        for table in &self.tables {
            out.push(render_table(dialect, table));
        }
        for index in &self.indexes {
            out.push(render_index(dialect, index));
        }
        for tidx in &self.typed_indexes {
            if let Some(sql) = render_typed_index(dialect, tidx) {
                out.push(sql);
            }
        }
        for vt in &self.virtual_tables {
            if let Some(sql) = render_virtual_table(dialect, vt) {
                out.push(sql);
            }
        }
        out
    }
}

fn render_table(dialect: Dialect, table: &Table) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in &table.columns {
        let mut def = format!("{} {}", dialect.quote_ident(c.name), dialect.column_type(c.ty));
        if c.not_null {
            def.push_str(" NOT NULL");
        }
        if let Some(d) = c.default {
            def.push_str(&format!(" DEFAULT {d}"));
        }
        parts.push(def);
    }
    if !table.primary_key.is_empty() {
        let cols =
            table.primary_key.iter().map(|c| dialect.quote_ident(c)).collect::<Vec<_>>().join(", ");
        parts.push(format!("PRIMARY KEY ({cols})"));
    }
    for fk in &table.foreign_keys {
        let cols = fk.columns.iter().map(|c| dialect.quote_ident(c)).collect::<Vec<_>>().join(", ");
        let ref_cols = fk
            .references_columns
            .iter()
            .map(|c| dialect.quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let mut def = format!(
            "FOREIGN KEY ({cols}) REFERENCES {} ({ref_cols})",
            dialect.quote_ident(fk.references_table)
        );
        if fk.on_delete_cascade {
            def.push_str(" ON DELETE CASCADE");
        }
        parts.push(def);
    }

    let name = dialect.quote_ident(table.name);
    let body = parts.join(", ");
    match dialect {
        Dialect::Sqlite | Dialect::Postgres => {
            format!("CREATE TABLE IF NOT EXISTS {name} ({body})")
        }
        // SQL Server has no IF NOT EXISTS on CREATE TABLE, so existence is a guard.
        Dialect::SqlServer => format!(
            "IF OBJECT_ID(N'{}', N'U') IS NULL CREATE TABLE {name} ({body});",
            escape_sql_string(table.name)
        ),
    }
}

fn render_index(dialect: Dialect, index: &Index) -> String {
    let cols = index.columns.iter().map(|c| dialect.quote_ident(c)).collect::<Vec<_>>().join(", ");
    let name = dialect.quote_ident(index.name);
    let table = dialect.quote_ident(index.table);
    match dialect {
        Dialect::Sqlite | Dialect::Postgres => {
            format!("CREATE INDEX IF NOT EXISTS {name} ON {table} ({cols})")
        }
        Dialect::SqlServer => format!(
            "IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = N'{}' \
             AND object_id = OBJECT_ID(N'{}')) CREATE INDEX {name} ON {table} ({cols});",
            escape_sql_string(index.name),
            escape_sql_string(index.table)
        ),
    }
}

/// Escape a single quoted SQL string literal. Only reached with our own static
/// table names, but a guard beats trusting that they stay quote free.
pub(super) fn escape_sql_string(s: &str) -> String {
    s.replace("'", "''")
}

fn render_typed_index(dialect: Dialect, tidx: &TypedIndex) -> Option<String> {
    let cols = tidx.columns.iter().map(|c| dialect.quote_ident(c)).collect::<Vec<_>>().join(", ");
    let name = dialect.quote_ident(tidx.name);
    let table = dialect.quote_ident(tidx.table);

    match (&tidx.kind, dialect) {
        // IvfFlat is a PostgreSQL-only index type: skip it on other engines.
        (IndexKind::IvfFlat, Dialect::Sqlite | Dialect::SqlServer) => None,
        (IndexKind::IvfFlat, Dialect::Postgres) => Some(format!(
            "CREATE INDEX IF NOT EXISTS {name} ON {table} USING ivfflat ({cols} vector_cosine_ops)"
        )),
        // GIN is PostgreSQL-specific; on other engines fall back to a standard index.
        (IndexKind::Gin, Dialect::Postgres) => {
            Some(format!("CREATE INDEX IF NOT EXISTS {name} ON {table} USING GIN ({cols})"))
        }
        (IndexKind::Standard | IndexKind::Gin, Dialect::Sqlite | Dialect::Postgres) => {
            Some(format!("CREATE INDEX IF NOT EXISTS {name} ON {table} ({cols})"))
        }
        (IndexKind::Standard | IndexKind::Gin, Dialect::SqlServer) => Some(format!(
            "IF NOT EXISTS (SELECT 1 FROM sys.indexes WHERE name = N'{}' \
             AND object_id = OBJECT_ID(N'{}')) CREATE INDEX {name} ON {table} ({cols});",
            escape_sql_string(tidx.name),
            escape_sql_string(tidx.table)
        )),
    }
}

fn render_virtual_table(dialect: Dialect, vt: &VirtualTableDef) -> Option<String> {
    // Skip if restricted to a different dialect.
    if vt.dialect.is_some_and(|d| d != dialect) {
        return None;
    }
    // Virtual tables are only meaningful on SQLite; skip on others unless explicitly allowed.
    if dialect != Dialect::Sqlite && vt.dialect.is_none() {
        return None;
    }
    let name = dialect.quote_ident(vt.name);
    let cols = vt.columns.join(", ");
    Some(format!("CREATE VIRTUAL TABLE IF NOT EXISTS {name} USING {} ({cols})", vt.using))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes() -> Table {
        Table::new(
            "nodes",
            vec![
                Column::required("volume_id", ColumnType::TextKey(200)),
                Column::required("path", ColumnType::TextKey(700)),
                Column::new("parent", ColumnType::Text),
                Column::required("size", ColumnType::BigInt).default("0"),
                Column::required("mtime", ColumnType::Double),
                Column::new("sha256", ColumnType::Text),
            ],
            vec!["volume_id", "path"],
        )
    }

    #[test]
    fn sqlite_ddl_keeps_todays_type_keywords() {
        let sql = render_table(Dialect::Sqlite, &nodes());
        assert!(sql.starts_with("CREATE TABLE IF NOT EXISTS \"nodes\" ("), "{sql}");
        assert!(sql.contains("\"path\" TEXT NOT NULL"), "{sql}");
        assert!(sql.contains("\"size\" INTEGER NOT NULL DEFAULT 0"), "{sql}");
        assert!(sql.contains("\"mtime\" REAL NOT NULL"), "{sql}");
        assert!(sql.contains("\"sha256\" TEXT"), "{sql}");
        assert!(sql.ends_with("PRIMARY KEY (\"volume_id\", \"path\"))"), "{sql}");
    }

    #[test]
    fn postgres_ddl_uses_portable_types() {
        let sql = render_table(Dialect::Postgres, &nodes());
        assert!(sql.contains("\"size\" BIGINT NOT NULL DEFAULT 0"), "{sql}");
        assert!(sql.contains("\"mtime\" DOUBLE PRECISION NOT NULL"), "{sql}");
    }

    /// The keyed columns must be sized, otherwise SQL Server rejects the PK.
    #[test]
    fn sqlserver_ddl_guards_existence_and_sizes_key_columns() {
        let sql = render_table(Dialect::SqlServer, &nodes());
        assert!(sql.starts_with("IF OBJECT_ID(N'nodes', N'U') IS NULL CREATE TABLE [nodes] ("));
        assert!(sql.contains("[volume_id] NVARCHAR(200) NOT NULL"), "{sql}");
        assert!(sql.contains("[path] NVARCHAR(700) NOT NULL"), "{sql}");
        // An unkeyed text column stays unbounded.
        assert!(sql.contains("[parent] NVARCHAR(MAX)"), "{sql}");
        assert!(sql.contains("[mtime] FLOAT NOT NULL"), "{sql}");
        assert!(sql.ends_with("PRIMARY KEY ([volume_id], [path]));"), "{sql}");
    }

    #[test]
    fn foreign_key_cascade_renders_on_every_engine() {
        let t = Table::new(
            "project_member",
            vec![
                Column::required("project_id", ColumnType::TextKey(200)),
                Column::required("person", ColumnType::TextKey(320)),
            ],
            vec!["project_id", "person"],
        )
        .foreign_key(ForeignKey {
            columns: vec!["project_id"],
            references_table: "project",
            references_columns: vec!["id"],
            on_delete_cascade: true,
        });
        for d in [Dialect::Sqlite, Dialect::Postgres] {
            let sql = render_table(d, &t);
            assert!(
                sql.contains(
                    "FOREIGN KEY (\"project_id\") REFERENCES \"project\" (\"id\") ON DELETE CASCADE"
                ),
                "{sql}"
            );
        }
        let sql = render_table(Dialect::SqlServer, &t);
        assert!(
            sql.contains(
                "FOREIGN KEY ([project_id]) REFERENCES [project] ([id]) ON DELETE CASCADE"
            ),
            "{sql}"
        );
    }

    #[test]
    fn index_ddl_is_idempotent_per_engine() {
        let idx = Index { name: "idx_nodes_parent", table: "nodes", columns: vec!["parent"] };
        assert_eq!(
            render_index(Dialect::Sqlite, &idx),
            "CREATE INDEX IF NOT EXISTS \"idx_nodes_parent\" ON \"nodes\" (\"parent\")"
        );
        assert_eq!(
            render_index(Dialect::Postgres, &idx),
            "CREATE INDEX IF NOT EXISTS \"idx_nodes_parent\" ON \"nodes\" (\"parent\")"
        );
        let mssql = render_index(Dialect::SqlServer, &idx);
        assert!(mssql.starts_with("IF NOT EXISTS (SELECT 1 FROM sys.indexes"), "{mssql}");
        assert!(
            mssql.contains("CREATE INDEX [idx_nodes_parent] ON [nodes] ([parent]);"),
            "{mssql}"
        );
    }

    #[test]
    fn recreate_if_missing_column_is_recorded_on_the_schema_set() {
        let s = SchemaSet::default().recreate_if_missing_column("oauth_tokens", "host");
        assert_eq!(
            s.table_recreations,
            vec![TableRecreation { table: "oauth_tokens", guard_column: "host" }]
        );
    }

    #[test]
    fn schema_set_renders_tables_before_indexes() {
        let set = SchemaSet::new(
            vec![nodes()],
            vec![Index { name: "idx_nodes_parent", table: "nodes", columns: vec!["parent"] }],
        );
        let stmts = set.render(Dialect::Sqlite);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("CREATE TABLE"), "{:?}", stmts[0]);
        assert!(stmts[1].contains("CREATE INDEX"), "{:?}", stmts[1]);
    }
}
