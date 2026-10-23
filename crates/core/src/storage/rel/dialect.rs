//! The things that actually differ between the supported engines:
//! placeholder syntax, upsert syntax and column types.
//!
//! Everything else in our SQL is portable, so keeping the differences in one
//! enum means a statement is written once and rendered per engine, instead of
//! being duplicated twice and drifting.

use super::schema::ColumnMigration;
use crate::errors::{Result, ToolError};

/// A supported relational engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

/// A portable column type. Rendered to the engine's own type keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Unbounded text.
    Text,
    /// 64 bit signed integer.
    BigInt,
    /// Double precision float.
    Double,
    /// Opaque bytes.
    Blob,
    /// Full-text search vector: TSVECTOR on PostgreSQL, TEXT on SQLite.
    Tsvector,
    /// Dense float vector of dimension N: VECTOR(N) on PostgreSQL, TEXT on SQLite.
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
    /// rather than by table name, for example `{target}.refcount + 1`, so the
    /// expression stays correct if the upsert is ever rendered under another name.
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
    /// SQLite and PostgreSQL both name it by table (`blob_refs.refcount`).
    pub const TARGET_TOKEN: &'static str = "{target}";

    /// Substitute [`Self::TARGET_TOKEN`] with the name of the existing row.
    fn resolve_target(self, sql: &str, target: &str) -> String {
        sql.replace(Self::TARGET_TOKEN, target)
    }

    /// The engine's placeholder for a 1 based parameter index.
    pub fn placeholder(self, index: usize) -> String {
        match self {
            Self::Sqlite => format!("?{index}"),
            Self::Postgres => format!("${index}"),
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
            (Self::Sqlite, ColumnType::Text) => "TEXT".into(),
            (Self::Sqlite, ColumnType::BigInt) => "INTEGER".into(),
            (Self::Sqlite, ColumnType::Double) => "REAL".into(),
            (Self::Sqlite, ColumnType::Blob) => "BLOB".into(),
            // On SQLite, tsvector and vector fall back to TEXT. Vector storage
            // uses the vec0 virtual table (see search/vector_sqlite.rs), not this column.
            (Self::Sqlite, ColumnType::Tsvector | ColumnType::Vector(_)) => "TEXT".into(),

            (Self::Postgres, ColumnType::Text) => "TEXT".into(),
            (Self::Postgres, ColumnType::BigInt) => "BIGINT".into(),
            (Self::Postgres, ColumnType::Double) => "DOUBLE PRECISION".into(),
            (Self::Postgres, ColumnType::Blob) => "BYTEA".into(),
            (Self::Postgres, ColumnType::Tsvector) => "TSVECTOR".into(),
            (Self::Postgres, ColumnType::Vector(n)) => format!("VECTOR({n})"),
        }
    }

    /// Quote an identifier so a reserved word such as `type` stays usable.
    pub fn quote_ident(self, name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    /// Render a page clause for a query that already has `ORDER BY` (paging without
    /// a stable order is meaningless on any engine).
    pub fn render_limit_offset(self, limit: i64, offset: i64) -> String {
        format!("LIMIT {limit} OFFSET {offset}")
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

        let conflict =
            upsert.conflict.iter().map(|c| self.quote_ident(c)).collect::<Vec<_>>().join(", ");
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
}

/// Render one [`ColumnMigration`] as this engine's `ALTER TABLE ... ADD COLUMN`.
///
/// PostgreSQL carries its own existence guard, so the statement is idempotent on
/// its own. SQLite has no `ADD COLUMN IF NOT EXISTS` and no way to
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Dialect; 2] = [Dialect::Sqlite, Dialect::Postgres];

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
    }

    /// Only SQLite renders an unguarded statement, which is why its `migrate`
    /// probes the column before running it.
    #[test]
    fn only_sqlite_renders_an_unguarded_column_migration() {
        let m = index_mode_migration();
        let sql = render_column_migration(Dialect::Postgres, &m);
        assert!(sql.contains("IF NOT EXISTS"), "postgres must guard: {sql}");
        assert!(!render_column_migration(Dialect::Sqlite, &m).contains("IF NOT EXISTS"));
    }

    #[test]
    fn placeholders_use_each_engines_syntax() {
        assert_eq!(Dialect::Sqlite.placeholder(1), "?1");
        assert_eq!(Dialect::Postgres.placeholder(1), "$1");
        // Two digit indices must survive: our widest insert binds ten columns.
        assert_eq!(Dialect::Sqlite.placeholder(10), "?10");
        assert_eq!(Dialect::Postgres.placeholder(10), "$10");
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
    }

    #[test]
    fn render_placeholders_handles_ten_or_more_parameters() {
        let sql = "VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)";
        assert_eq!(
            Dialect::Postgres.render_placeholders(sql).unwrap(),
            "VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
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
            (ColumnType::Text, "TEXT", "TEXT"),
            (ColumnType::BigInt, "INTEGER", "BIGINT"),
            (ColumnType::Double, "REAL", "DOUBLE PRECISION"),
            (ColumnType::Blob, "BLOB", "BYTEA"),
        ];
        for (ty, sqlite, postgres) in cases {
            assert_eq!(Dialect::Sqlite.column_type(ty), sqlite, "{ty:?}");
            assert_eq!(Dialect::Postgres.column_type(ty), postgres, "{ty:?}");
        }
    }

    #[test]
    fn tsvector_and_vector_render_per_engine() {
        assert_eq!(Dialect::Postgres.column_type(ColumnType::Tsvector), "TSVECTOR");
        assert_eq!(Dialect::Postgres.column_type(ColumnType::Vector(1536)), "VECTOR(1536)");
        // SQLite falls back to TEXT.
        assert_eq!(Dialect::Sqlite.column_type(ColumnType::Tsvector), "TEXT");
        assert_eq!(Dialect::Sqlite.column_type(ColumnType::Vector(768)), "TEXT");
    }

    #[test]
    fn identifiers_are_quoted_per_engine() {
        // 'type' is a git_objects column name and a reserved word.
        assert_eq!(Dialect::Sqlite.quote_ident("type"), "\"type\"");
        assert_eq!(Dialect::Postgres.quote_ident("type"), "\"type\"");
    }

    #[test]
    fn table_columns_query_uses_each_engines_introspection() {
        assert!(Dialect::Sqlite.table_columns_query().contains("pragma_table_info"));
        assert!(Dialect::Postgres.table_columns_query().contains("information_schema.columns"));
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

    /// '[' is no wildcard on SQLite or PostgreSQL: escaping it would produce an
    /// invalid escape sequence rather than a literal bracket.
    #[test]
    fn spec_0051_like_escaping_leaves_a_bracket_alone() {
        assert_eq!(Dialect::Sqlite.escape_like_literal("a[b]"), "a[b]");
        assert_eq!(Dialect::Postgres.escape_like_literal("a[b]"), "a[b]");
    }

    #[test]
    fn limit_offset_is_portable_across_engines() {
        assert_eq!(Dialect::Sqlite.render_limit_offset(10, 0), "LIMIT 10 OFFSET 0");
        assert_eq!(Dialect::Postgres.render_limit_offset(10, 5), "LIMIT 10 OFFSET 5");
    }

    #[test]
    fn spec_0051_upsert_ignore_renders_do_nothing() {
        let u = Upsert::ignore("nodes", vec!["path", "name"], vec!["path"]);
        assert_eq!(
            Dialect::Sqlite.render_upsert(&u),
            "INSERT INTO \"nodes\" (\"path\", \"name\") VALUES (?1, ?2) \
             ON CONFLICT (\"path\") DO NOTHING"
        );
        // Postgres shares the ON CONFLICT form, and an upsert is emitted with
        // canonical placeholders like any other statement.
        assert_eq!(Dialect::Postgres.render_upsert(&u), Dialect::Sqlite.render_upsert(&u));
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
