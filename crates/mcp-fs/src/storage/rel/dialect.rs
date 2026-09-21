//! The three things that actually differ between the supported engines:
//! placeholder syntax, upsert syntax and column types.
//!
//! Everything else in our SQL is portable, so keeping the differences in one
//! enum means a statement is written once and rendered per engine, instead of
//! being duplicated three times and drifting.

use super::schema::{ColumnMigration, escape_sql_string};
use crate::errors::{Result, ToolError};

/// A supported relational engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Sqlite,
    Postgres,
    SqlServer,
}

/// A portable column type. Rendered to the engine's own type keyword.
///
/// `TextKey` exists because SQL Server cannot index `NVARCHAR(MAX)`, so a text
/// column that participates in a primary key or an index must carry a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Unbounded text.
    Text,
    /// Text that is part of a key or an index, with a maximum length.
    TextKey(u32),
    /// 64 bit signed integer.
    BigInt,
    /// Double precision float.
    Double,
    /// Opaque bytes.
    Blob,
    /// Full-text search vector: TSVECTOR on PostgreSQL, TEXT on other engines.
    Tsvector,
    /// Dense float vector of dimension N: VECTOR(N) on PostgreSQL, TEXT elsewhere.
    /// On SQLite the actual vector storage uses a vec0 virtual table, not this column.
    Vector(u32),
}

/// One assignment applied when an upsert hits an existing row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assign {
    pub column: &'static str,
    pub value: AssignValue,
}

impl Assign {
    /// Overwrite the column with the value that was being inserted.
    pub fn inserted(column: &'static str) -> Self {
        Self { column, value: AssignValue::Inserted }
    }

    /// Compute the column from raw SQL, evaluated against the existing row.
    ///
    /// Refer to the existing row through the [`Dialect::TARGET_TOKEN`] placeholder
    /// rather than by table name, for example `{target}.refcount + 1`. Each engine
    /// names that row differently (the table itself on SQLite and PostgreSQL, the
    /// `MERGE` alias on SQL Server), and a hard coded table name renders SQL that
    /// one engine silently rejects.
    pub fn expr(column: &'static str, sql: &'static str) -> Self {
        Self { column, value: AssignValue::Expr(sql) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignValue {
    /// The value from the failed insert.
    Inserted,
    /// Raw SQL evaluated against the existing row, referring to it through
    /// [`Dialect::TARGET_TOKEN`].
    Expr(&'static str),
}

/// What to do when an insert collides with an existing row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpsertAction {
    /// Leave the existing row untouched.
    Nothing,
    /// Apply these assignments to the existing row.
    Update(Vec<Assign>),
}

/// An insert that must tolerate an existing row, declared once and rendered per
/// dialect. Replaces the SQLite only `INSERT OR IGNORE` and `INSERT OR REPLACE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upsert {
    pub table: &'static str,
    /// Every inserted column, in placeholder order.
    pub columns: Vec<&'static str>,
    /// The columns that detect the collision, normally the primary key.
    pub conflict: Vec<&'static str>,
    pub action: UpsertAction,
}

impl Upsert {
    /// Insert, and on collision keep the existing row.
    pub fn ignore(
        table: &'static str,
        columns: Vec<&'static str>,
        conflict: Vec<&'static str>,
    ) -> Self {
        Self { table, columns, conflict, action: UpsertAction::Nothing }
    }

    /// Insert, and on collision overwrite every column that is not part of the
    /// conflict key. This is what `INSERT OR REPLACE` meant at our call sites.
    pub fn replace(
        table: &'static str,
        columns: Vec<&'static str>,
        conflict: Vec<&'static str>,
    ) -> Self {
        let sets =
            columns.iter().filter(|c| !conflict.contains(c)).map(|c| Assign::inserted(c)).collect();
        Self { table, columns, conflict, action: UpsertAction::Update(sets) }
    }

    /// Insert, and on collision apply explicit assignments.
    pub fn update(
        table: &'static str,
        columns: Vec<&'static str>,
        conflict: Vec<&'static str>,
        sets: Vec<Assign>,
    ) -> Self {
        Self { table, columns, conflict, action: UpsertAction::Update(sets) }
    }
}

impl Dialect {
    /// The placeholder an [`AssignValue::Expr`] uses to name the existing row.
    ///
    /// SQLite and PostgreSQL name it by table (`blob_refs.refcount`), while a SQL
    /// Server `MERGE` aliases its target and rejects a base table reference once
    /// aliased. One token, substituted per dialect, keeps the expression portable.
    pub const TARGET_TOKEN: &'static str = "{target}";

    /// Substitute [`Self::TARGET_TOKEN`] with how this engine names the existing row.
    fn resolve_target(self, sql: &str, target: &str) -> String {
        sql.replace(Self::TARGET_TOKEN, target)
    }

    /// The engine's placeholder for a 1 based parameter index.
    pub fn placeholder(self, index: usize) -> String {
        match self {
            Self::Sqlite => format!("?{index}"),
            Self::Postgres => format!("${index}"),
            Self::SqlServer => format!("@P{index}"),
        }
    }

    /// Rewrite the canonical `?N` placeholders of `sql` into this dialect's form.
    ///
    /// Text inside single quoted literals is left alone, so a literal containing
    /// a question mark can never be mistaken for a parameter.
    ///
    /// Fails on an unparsable index rather than guessing one: a wrong index binds
    /// the wrong parameter, which is a silent data error instead of a loud one.
    pub fn render_placeholders(self, sql: &str) -> Result<String> {
        let mut out = String::with_capacity(sql.len());
        let mut chars = sql.char_indices().peekable();
        let mut in_literal = false;
        while let Some((_, c)) = chars.next() {
            if in_literal {
                out.push(c);
                // '' inside a literal is an escaped quote, not the end of it.
                if c == '\'' {
                    if let Some(&(_, '\'')) = chars.peek() {
                        out.push('\'');
                        chars.next();
                    } else {
                        in_literal = false;
                    }
                }
                continue;
            }
            if c == '\'' {
                in_literal = true;
                out.push(c);
                continue;
            }
            if c == '?' {
                let mut digits = String::new();
                while let Some(&(_, d)) = chars.peek() {
                    if d.is_ascii_digit() {
                        digits.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if digits.is_empty() {
                    // A bare '?' is not our canonical form, so pass it through.
                    out.push(c);
                } else {
                    let index: usize = digits.parse().map_err(|_| {
                        ToolError::internal(format!(
                            "placeholder index '?{digits}' is not a usable parameter number"
                        ))
                    })?;
                    if index == 0 {
                        return Err(ToolError::internal(
                            "placeholder '?0' is invalid, parameter indexes are one based",
                        ));
                    }
                    out.push_str(&self.placeholder(index));
                }
                continue;
            }
            out.push(c);
        }
        Ok(out)
    }

    /// The engine keyword for a portable column type.
    pub fn column_type(self, ty: ColumnType) -> String {
        match (self, ty) {
            (Self::Sqlite, ColumnType::Text | ColumnType::TextKey(_)) => "TEXT".into(),
            (Self::Sqlite, ColumnType::BigInt) => "INTEGER".into(),
            (Self::Sqlite, ColumnType::Double) => "REAL".into(),
            (Self::Sqlite, ColumnType::Blob) => "BLOB".into(),
            // On SQLite, tsvector and vector fall back to TEXT. Vector storage
            // uses the vec0 virtual table (see search/vector_sqlite.rs), not this column.
            (Self::Sqlite, ColumnType::Tsvector | ColumnType::Vector(_)) => "TEXT".into(),

            (Self::Postgres, ColumnType::Text | ColumnType::TextKey(_)) => "TEXT".into(),
            (Self::Postgres, ColumnType::BigInt) => "BIGINT".into(),
            (Self::Postgres, ColumnType::Double) => "DOUBLE PRECISION".into(),
            (Self::Postgres, ColumnType::Blob) => "BYTEA".into(),
            (Self::Postgres, ColumnType::Tsvector) => "TSVECTOR".into(),
            (Self::Postgres, ColumnType::Vector(n)) => format!("VECTOR({n})"),

            (Self::SqlServer, ColumnType::Text) => "NVARCHAR(MAX)".into(),
            // A keyed text column must be sized: SQL Server refuses to index MAX.
            (Self::SqlServer, ColumnType::TextKey(n)) => format!("NVARCHAR({n})"),
            (Self::SqlServer, ColumnType::BigInt) => "BIGINT".into(),
            (Self::SqlServer, ColumnType::Double) => "FLOAT".into(),
            (Self::SqlServer, ColumnType::Blob) => "VARBINARY(MAX)".into(),
            // SQL Server has no tsvector or pgvector; both degrade to TEXT.
            (Self::SqlServer, ColumnType::Tsvector | ColumnType::Vector(_)) => {
                "NVARCHAR(MAX)".into()
            }
        }
    }

    /// Quote an identifier so a reserved word such as `type` stays usable.
    pub fn quote_ident(self, name: &str) -> String {
        match self {
            Self::Sqlite | Self::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
            Self::SqlServer => format!("[{}]", name.replace(']', "]]")),
        }
    }

    /// The engine's string length function, for `ORDER BY`.
    pub fn length_fn(self) -> &'static str {
        match self {
            Self::Sqlite | Self::Postgres => "length",
            Self::SqlServer => "LEN",
        }
    }

    /// A query returning one text column: the name of every table.
    pub fn table_names_query(self) -> &'static str {
        match self {
            Self::Sqlite => {
                "SELECT name FROM sqlite_master WHERE type='table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name"
            }
            Self::Postgres => {
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = current_schema() ORDER BY table_name"
            }
            Self::SqlServer => "SELECT name FROM sys.tables WHERE type = 'U' ORDER BY name",
        }
    }

    /// A query returning one text column: every column name of the table bound
    /// as `?1`, empty when the table does not exist. Powers the DRIFT-005 guard
    /// ([`super::apply_table_recreations`]) without engine specific code at the
    /// call site: SQLite already probes `pragma_table_info` the same way for an
    /// `ADD COLUMN` guard, so a missing table there is simply zero rows too.
    pub fn table_columns_query(self) -> &'static str {
        match self {
            Self::Sqlite => "SELECT name FROM pragma_table_info(?1)",
            Self::Postgres => {
                "SELECT column_name FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = ?1"
            }
            Self::SqlServer => "SELECT name FROM sys.columns WHERE object_id = OBJECT_ID(?1)",
        }
    }

    /// The escape character our `LIKE` patterns use. Paired with
    /// [`Self::escape_like_literal`], which is the only way to build a pattern.
    pub const LIKE_ESCAPE: char = '\\';

    /// Escape `literal` so it matches itself inside a `LIKE` pattern.
    ///
    /// The caller appends its own wildcard and must add `ESCAPE '\'` to the SQL.
    /// Without this a path holding `%` or `_` silently matches sibling rows,
    /// which is how an unescaped prefix over deletes a subtree.
    pub fn escape_like_literal(self, literal: &str) -> String {
        let mut out = String::with_capacity(literal.len());
        for c in literal.chars() {
            match c {
                // The escape char itself, then the two SQL standard wildcards.
                '\\' | '%' | '_' => {
                    out.push(Self::LIKE_ESCAPE);
                    out.push(c);
                }
                // SQL Server additionally treats '[' as a character class opener.
                // Escaping it on the other engines would be an invalid sequence.
                '[' if self == Self::SqlServer => {
                    out.push(Self::LIKE_ESCAPE);
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// Render an [`Upsert`] with `?N` placeholders in `columns` order.
    ///
    /// The returned SQL still carries canonical placeholders, so it goes through
    /// [`Self::render_placeholders`] like any other statement.
    pub fn render_upsert(self, upsert: &Upsert) -> String {
        let table = self.quote_ident(upsert.table);
        let cols =
            upsert.columns.iter().map(|c| self.quote_ident(c)).collect::<Vec<_>>().join(", ");
        let values =
            (1..=upsert.columns.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");

        match self {
            Self::Sqlite | Self::Postgres => {
                let conflict = upsert
                    .conflict
                    .iter()
                    .map(|c| self.quote_ident(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let action = match &upsert.action {
                    UpsertAction::Nothing => "DO NOTHING".to_string(),
                    UpsertAction::Update(sets) => {
                        let assignments = sets
                            .iter()
                            .map(|a| {
                                let col = self.quote_ident(a.column);
                                match &a.value {
                                    AssignValue::Inserted => {
                                        format!("{col} = excluded.{col}")
                                    }
                                    AssignValue::Expr(sql) => {
                                        format!("{col} = {}", self.resolve_target(sql, &table))
                                    }
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("DO UPDATE SET {assignments}")
                    }
                };
                format!(
                    "INSERT INTO {table} ({cols}) VALUES ({values}) \
                     ON CONFLICT ({conflict}) {action}"
                )
            }
            Self::SqlServer => {
                // HOLDLOCK is required: without it MERGE races a concurrent
                // insert and raises a duplicate key error instead of updating.
                let on = upsert
                    .conflict
                    .iter()
                    .map(|c| {
                        let col = self.quote_ident(c);
                        format!("tgt.{col} = src.{col}")
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let insert_values = upsert
                    .columns
                    .iter()
                    .map(|c| format!("src.{}", self.quote_ident(c)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let matched = match &upsert.action {
                    UpsertAction::Nothing => String::new(),
                    UpsertAction::Update(sets) => {
                        let assignments = sets
                            .iter()
                            .map(|a| {
                                let col = self.quote_ident(a.column);
                                match &a.value {
                                    AssignValue::Inserted => format!("{col} = src.{col}"),
                                    AssignValue::Expr(sql) => {
                                        format!("{col} = {}", self.resolve_target(sql, "tgt"))
                                    }
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(" WHEN MATCHED THEN UPDATE SET {assignments}")
                    }
                };
                format!(
                    "MERGE {table} WITH (HOLDLOCK) AS tgt \
                     USING (VALUES ({values})) AS src ({cols}) ON {on}{matched} \
                     WHEN NOT MATCHED THEN INSERT ({cols}) VALUES ({insert_values});"
                )
            }
        }
    }
}

/// Render one [`ColumnMigration`] as this engine's `ALTER TABLE ... ADD COLUMN`.
///
/// PostgreSQL and SQL Server carry their own existence guard, so the statement is
/// idempotent on its own. SQLite has no `ADD COLUMN IF NOT EXISTS` and no way to
/// guard DDL inside a single statement, so its form is bare and the caller
/// ([`super::SqliteRelationalDb::migrate`]) probes the column first.
pub fn render_column_migration(dialect: Dialect, m: &ColumnMigration) -> String {
    let table = dialect.quote_ident(m.table);
    let column = dialect.quote_ident(m.column);
    let ty = dialect.column_type(m.ty);
    let not_null = if m.not_null { " NOT NULL" } else { "" };
    let default = m.default;
    match dialect {
        Dialect::Sqlite => {
            format!("ALTER TABLE {table} ADD COLUMN {column} {ty}{not_null} DEFAULT {default}")
        }
        Dialect::Postgres => format!(
            "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {column} {ty}{not_null} DEFAULT {default}"
        ),
        Dialect::SqlServer => format!(
            "IF NOT EXISTS (SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID(N'{}') \
             AND name = N'{}') ALTER TABLE {table} ADD {column} {ty}{not_null} DEFAULT {default};",
            escape_sql_string(m.table),
            escape_sql_string(m.column)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Dialect; 3] = [Dialect::Sqlite, Dialect::Postgres, Dialect::SqlServer];

    fn index_mode_migration() -> ColumnMigration {
        ColumnMigration {
            table: "project",
            column: "index_mode",
            ty: ColumnType::Text,
            not_null: true,
            default: "'none'",
        }
    }

    #[test]
    fn column_migration_renders_per_engine() {
        let m = index_mode_migration();
        assert_eq!(
            render_column_migration(Dialect::Sqlite, &m),
            "ALTER TABLE \"project\" ADD COLUMN \"index_mode\" TEXT NOT NULL DEFAULT 'none'"
        );
        assert_eq!(
            render_column_migration(Dialect::Postgres, &m),
            "ALTER TABLE \"project\" ADD COLUMN IF NOT EXISTS \"index_mode\" \
             TEXT NOT NULL DEFAULT 'none'"
        );
        let mssql = render_column_migration(Dialect::SqlServer, &m);
        assert!(mssql.starts_with("IF NOT EXISTS (SELECT 1 FROM sys.columns"), "{mssql}");
        assert!(
            mssql.contains(
                "ALTER TABLE [project] ADD [index_mode] NVARCHAR(MAX) NOT NULL DEFAULT 'none';"
            ),
            "{mssql}"
        );
    }

    /// Only SQLite renders an unguarded statement, which is why its `migrate`
    /// probes the column before running it.
    #[test]
    fn only_sqlite_renders_an_unguarded_column_migration() {
        let m = index_mode_migration();
        for d in [Dialect::Postgres, Dialect::SqlServer] {
            let sql = render_column_migration(d, &m);
            assert!(sql.contains("IF NOT EXISTS"), "{d:?} must guard: {sql}");
        }
        assert!(!render_column_migration(Dialect::Sqlite, &m).contains("IF NOT EXISTS"));
    }

    #[test]
    fn placeholders_use_each_engines_syntax() {
        assert_eq!(Dialect::Sqlite.placeholder(1), "?1");
        assert_eq!(Dialect::Postgres.placeholder(1), "$1");
        assert_eq!(Dialect::SqlServer.placeholder(1), "@P1");
        // Two digit indices must survive: our widest insert binds ten columns.
        assert_eq!(Dialect::Sqlite.placeholder(10), "?10");
        assert_eq!(Dialect::Postgres.placeholder(10), "$10");
        assert_eq!(Dialect::SqlServer.placeholder(10), "@P10");
    }

    #[test]
    fn render_placeholders_rewrites_the_canonical_form() {
        let sql = "SELECT a FROM t WHERE p=?1 AND q=?2";
        assert_eq!(
            Dialect::Sqlite.render_placeholders(sql).unwrap(),
            "SELECT a FROM t WHERE p=?1 AND q=?2"
        );
        assert_eq!(
            Dialect::Postgres.render_placeholders(sql).unwrap(),
            "SELECT a FROM t WHERE p=$1 AND q=$2"
        );
        assert_eq!(
            Dialect::SqlServer.render_placeholders(sql).unwrap(),
            "SELECT a FROM t WHERE p=@P1 AND q=@P2"
        );
    }

    #[test]
    fn render_placeholders_handles_ten_or_more_parameters() {
        let sql = "VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)";
        assert_eq!(
            Dialect::Postgres.render_placeholders(sql).unwrap(),
            "VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
        );
        assert_eq!(
            Dialect::SqlServer.render_placeholders(sql).unwrap(),
            "VALUES(@P1,@P2,@P3,@P4,@P5,@P6,@P7,@P8,@P9,@P10,@P11)"
        );
    }

    /// A '?' inside a quoted literal is data, not a parameter. Rewriting it
    /// would corrupt the value and shift every later parameter index.
    #[test]
    fn render_placeholders_ignores_quoted_literals() {
        let sql = "SELECT ?1 WHERE kind='what?2' AND x=?2";
        assert_eq!(
            Dialect::Postgres.render_placeholders(sql).unwrap(),
            "SELECT $1 WHERE kind='what?2' AND x=$2"
        );
    }

    #[test]
    fn render_placeholders_handles_doubled_quotes_in_a_literal() {
        let sql = "SELECT ?1 WHERE n='it''s ?9' AND m=?2";
        assert_eq!(
            Dialect::Postgres.render_placeholders(sql).unwrap(),
            "SELECT $1 WHERE n='it''s ?9' AND m=$2"
        );
    }

    /// An index we cannot parse must fail loudly. Binding parameter 0 in a one
    /// based scheme would silently read the wrong value.
    #[test]
    fn render_placeholders_rejects_an_unusable_index() {
        for d in ALL {
            let zero = d.render_placeholders("SELECT ?0").expect_err("?0 is invalid");
            assert_eq!(zero.code, crate::errors::code::INTERNAL_ERROR);
            assert!(zero.message.contains("one based"), "{}", zero.message);

            let huge = format!("SELECT ?{}", "9".repeat(40));
            let overflow = d.render_placeholders(&huge).expect_err("overflow is invalid");
            assert!(overflow.message.contains("not a usable"), "{}", overflow.message);
        }
    }

    /// A bare '?' is not our canonical form and must survive untouched, because
    /// it is legal inside an operator such as the Postgres `?` json test.
    #[test]
    fn render_placeholders_passes_a_bare_question_mark_through() {
        assert_eq!(
            Dialect::Postgres.render_placeholders("SELECT a ? b, ?1").unwrap(),
            "SELECT a ? b, $1"
        );
    }

    #[test]
    fn column_types_map_per_engine() {
        let cases = [
            (ColumnType::Text, "TEXT", "TEXT", "NVARCHAR(MAX)"),
            (ColumnType::TextKey(400), "TEXT", "TEXT", "NVARCHAR(400)"),
            (ColumnType::BigInt, "INTEGER", "BIGINT", "BIGINT"),
            (ColumnType::Double, "REAL", "DOUBLE PRECISION", "FLOAT"),
            (ColumnType::Blob, "BLOB", "BYTEA", "VARBINARY(MAX)"),
        ];
        for (ty, sqlite, postgres, sqlserver) in cases {
            assert_eq!(Dialect::Sqlite.column_type(ty), sqlite, "{ty:?}");
            assert_eq!(Dialect::Postgres.column_type(ty), postgres, "{ty:?}");
            assert_eq!(Dialect::SqlServer.column_type(ty), sqlserver, "{ty:?}");
        }
    }

    #[test]
    fn tsvector_and_vector_render_per_engine() {
        assert_eq!(Dialect::Postgres.column_type(ColumnType::Tsvector), "TSVECTOR");
        assert_eq!(Dialect::Postgres.column_type(ColumnType::Vector(1536)), "VECTOR(1536)");
        // Non-PostgreSQL engines fall back to TEXT / NVARCHAR(MAX).
        assert_eq!(Dialect::Sqlite.column_type(ColumnType::Tsvector), "TEXT");
        assert_eq!(Dialect::Sqlite.column_type(ColumnType::Vector(768)), "TEXT");
        assert_eq!(Dialect::SqlServer.column_type(ColumnType::Tsvector), "NVARCHAR(MAX)");
        assert_eq!(Dialect::SqlServer.column_type(ColumnType::Vector(512)), "NVARCHAR(MAX)");
    }

    #[test]
    fn identifiers_are_quoted_per_engine() {
        // 'type' is a git_objects column name and a reserved word on some engines.
        assert_eq!(Dialect::Sqlite.quote_ident("type"), "\"type\"");
        assert_eq!(Dialect::Postgres.quote_ident("type"), "\"type\"");
        assert_eq!(Dialect::SqlServer.quote_ident("type"), "[type]");
    }

    #[test]
    fn length_function_differs_on_sqlserver() {
        assert_eq!(Dialect::Sqlite.length_fn(), "length");
        assert_eq!(Dialect::Postgres.length_fn(), "length");
        assert_eq!(Dialect::SqlServer.length_fn(), "LEN");
    }

    #[test]
    fn table_columns_query_uses_each_engines_introspection() {
        assert!(Dialect::Sqlite.table_columns_query().contains("pragma_table_info"));
        assert!(Dialect::Postgres.table_columns_query().contains("information_schema.columns"));
        assert!(Dialect::SqlServer.table_columns_query().contains("sys.columns"));
        for d in ALL {
            assert!(d.table_columns_query().contains("?1"), "{d:?} must bind the table name");
        }
    }

    #[test]
    fn like_escaping_covers_both_sql_wildcards() {
        for d in ALL {
            assert_eq!(d.escape_like_literal("/a/b"), "/a/b", "{d:?} leaves plain text");
            assert_eq!(d.escape_like_literal("50%"), "50\\%", "{d:?} escapes percent");
            assert_eq!(d.escape_like_literal("a_b"), "a\\_b", "{d:?} escapes underscore");
            assert_eq!(d.escape_like_literal("a\\b"), "a\\\\b", "{d:?} escapes the escape char");
        }
    }

    /// '[' opens a character class on SQL Server only. Escaping it elsewhere
    /// would produce an invalid escape sequence rather than a literal bracket.
    #[test]
    fn like_escaping_of_bracket_is_sqlserver_only() {
        assert_eq!(Dialect::Sqlite.escape_like_literal("a[b]"), "a[b]");
        assert_eq!(Dialect::Postgres.escape_like_literal("a[b]"), "a[b]");
        assert_eq!(Dialect::SqlServer.escape_like_literal("a[b]"), "a\\[b]");
    }

    #[test]
    fn upsert_ignore_renders_do_nothing_and_a_bare_merge() {
        let u = Upsert::ignore("nodes", vec!["path", "name"], vec!["path"]);
        assert_eq!(
            Dialect::Sqlite.render_upsert(&u),
            "INSERT INTO \"nodes\" (\"path\", \"name\") VALUES (?1, ?2) \
             ON CONFLICT (\"path\") DO NOTHING"
        );
        // Postgres shares the ON CONFLICT form, and an upsert is emitted with
        // canonical placeholders like any other statement.
        assert_eq!(Dialect::Postgres.render_upsert(&u), Dialect::Sqlite.render_upsert(&u));
        // No WHEN MATCHED clause, so an existing row is left untouched.
        let merged = Dialect::SqlServer.render_upsert(&u);
        assert!(!merged.contains("WHEN MATCHED"), "{merged}");
        assert!(merged.starts_with("MERGE [nodes] WITH (HOLDLOCK) AS tgt"), "{merged}");
        assert!(merged.contains("ON tgt.[path] = src.[path]"), "{merged}");
    }

    #[test]
    fn upsert_replace_updates_every_non_key_column() {
        let u = Upsert::replace("git_objects", vec!["hash", "type", "size"], vec!["hash"]);
        let sqlite = Dialect::Sqlite.render_upsert(&u);
        assert!(
            sqlite.contains(
                "ON CONFLICT (\"hash\") DO UPDATE SET \
                 \"type\" = excluded.\"type\", \"size\" = excluded.\"size\""
            ),
            "{sqlite}"
        );
        // The conflict key is never reassigned.
        assert!(!sqlite.contains("\"hash\" = excluded"), "{sqlite}");

        let mssql = Dialect::SqlServer.render_upsert(&u);
        assert!(
            mssql.contains("WHEN MATCHED THEN UPDATE SET [type] = src.[type], [size] = src.[size]"),
            "{mssql}"
        );
        assert!(
            mssql.contains(
                "WHEN NOT MATCHED THEN INSERT ([hash], [type], [size]) \
                 VALUES (src.[hash], src.[type], src.[size]);"
            ),
            "{mssql}"
        );
    }

    /// The refcount increment must read the existing row, so it is raw SQL
    /// rather than the inserted value.
    #[test]
    fn upsert_can_update_from_an_expression() {
        let u = Upsert::update(
            "blob_refs",
            vec!["sha256", "refcount", "size"],
            vec!["sha256"],
            vec![Assign::expr("refcount", "{target}.refcount + 1")],
        );
        // SQLite and PostgreSQL name the existing row by table.
        assert!(
            Dialect::Sqlite
                .render_upsert(&u)
                .contains("DO UPDATE SET \"refcount\" = \"blob_refs\".refcount + 1")
        );
        assert!(
            Dialect::Postgres
                .render_upsert(&u)
                .contains("DO UPDATE SET \"refcount\" = \"blob_refs\".refcount + 1")
        );
        // SQL Server aliases the MERGE target and REJECTS a base table reference
        // once aliased, so the expression must resolve to the alias instead.
        let mssql = Dialect::SqlServer.render_upsert(&u);
        assert!(
            mssql.contains("WHEN MATCHED THEN UPDATE SET [refcount] = tgt.refcount + 1"),
            "{mssql}"
        );
        assert!(
            !mssql.contains("blob_refs.refcount"),
            "an aliased MERGE cannot reference the base table: {mssql}"
        );
    }

    /// The token is the only supported way to name the existing row, so a stray
    /// one must never survive into rendered SQL on any engine.
    #[test]
    fn no_dialect_leaves_the_target_token_unresolved() {
        let u = Upsert::update(
            "t",
            vec!["k", "n"],
            vec!["k"],
            vec![Assign::expr("n", "{target}.n + 1")],
        );
        for d in ALL {
            let sql = d.render_upsert(&u);
            assert!(!sql.contains(Dialect::TARGET_TOKEN), "{d:?} left the token in: {sql}");
        }
    }

    #[test]
    fn upsert_placeholders_render_for_the_target_dialect() {
        let u = Upsert::replace("t", vec!["a", "b"], vec!["a"]);
        let rendered =
            Dialect::Postgres.render_placeholders(&Dialect::Postgres.render_upsert(&u)).unwrap();
        assert!(rendered.contains("VALUES ($1, $2)"), "{rendered}");
    }
}
