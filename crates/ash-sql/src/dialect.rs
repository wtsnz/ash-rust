use ash_core::{AttrType, AttributeDef, IdentityDef};

/// Where a text filter looks for its needle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMatch {
    Contains,
    StartsWith,
    EndsWith,
    /// The needle is a `LIKE` pattern of its own, used as given.
    Like,
}

/// `LIKE` pattern with `\`, `%`, and `_` in `needle` escaped by a backslash.
pub fn like_pattern(kind: TextMatch, needle: &str) -> String {
    if kind == TextMatch::Like {
        return needle.to_string();
    }
    let mut escaped = String::with_capacity(needle.len());
    for ch in needle.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    wrap_pattern(kind, &escaped, "%")
}

/// The `GLOB` pattern matching what the `LIKE` `pattern` does, case for case.
fn like_to_glob(pattern: &str) -> String {
    let mut glob = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    let literal = |glob: &mut String, ch: char| {
        if matches!(ch, '*' | '?' | '[') {
            glob.push('[');
            glob.push(ch);
            glob.push(']');
        } else {
            glob.push(ch);
        }
    };
    while let Some(ch) = chars.next() {
        match ch {
            '%' => glob.push('*'),
            '_' => glob.push('?'),
            '\\' => literal(&mut glob, chars.next().unwrap_or('\\')),
            other => literal(&mut glob, other),
        }
    }
    glob
}

/// SQLite `GLOB` pattern with `*`, `?`, and `[` in `needle` matched literally.
pub fn glob_pattern(kind: TextMatch, needle: &str) -> String {
    if kind == TextMatch::Like {
        return like_to_glob(needle);
    }
    let mut escaped = String::with_capacity(needle.len());
    for ch in needle.chars() {
        match ch {
            '*' | '?' | '[' => {
                escaped.push('[');
                escaped.push(ch);
                escaped.push(']');
            }
            _ => escaped.push(ch),
        }
    }
    wrap_pattern(kind, &escaped, "*")
}

fn wrap_pattern(kind: TextMatch, escaped: &str, any: &str) -> String {
    match kind {
        TextMatch::Contains => format!("{any}{escaped}{any}"),
        TextMatch::StartsWith => format!("{escaped}{any}"),
        TextMatch::EndsWith => format!("{any}{escaped}"),
        TextMatch::Like => escaped.to_string(),
    }
}

/// Defines database dialect-specific SQL syntax rules, quoting, placeholders, and types.
pub trait SqlDialect: Send + Sync + 'static {
    /// Database identifier name (e.g. "postgres", "sqlite").
    fn name(&self) -> &'static str;

    /// Quote an identifier (e.g. `"users"`).
    fn quote_identifier(&self, ident: &str) -> String {
        format!("\"{}\"", ident.replace('"', "\"\""))
    }

    /// `table` in the quoted `schema`, where a context-tenant resource keeps a tenant's
    /// rows, or `None` for a database without schemas, which can't keep tenants apart.
    fn qualify_table(&self, _schema: &str, _table: &str) -> Option<String> {
        None
    }

    /// SQL parameter placeholder (e.g. `$1` for Postgres, `?` for SQLite).
    fn placeholder(&self, index: usize) -> String;

    /// Cast a bound placeholder when the column rejects an untyped string parameter.
    fn cast_param(&self, _ty: AttrType, placeholder: &str) -> String {
        placeholder.to_string()
    }

    /// Cast a computed expression to the column type of `ty`, for SQL whose result type
    /// differs from what was declared (Postgres sums `bigint` as `numeric`).
    fn cast_expression(&self, _ty: AttrType, expression: &str) -> String {
        expression.to_string()
    }

    /// SQL literal for a standard-base64 binary value.
    fn binary_literal(&self, encoded: &str) -> String {
        let Ok(binary) = ash_core::Binary::parse(encoded) else {
            return "NULL".to_string();
        };
        let hex: String = binary
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
        format!("X'{hex}'")
    }

    /// Map Ash [`AttributeDef`] to a native SQL column type string.
    fn column_type(&self, attr: &AttributeDef) -> String;

    /// Emits `ON CONFLICT (...) DO UPDATE` clause.
    /// `ON CONFLICT` on `identity`, whose unique index covers `columns`
    /// ([`IdentityDef::columns`]).
    fn upsert_clause(&self, identity: &IdentityDef, columns: &[&str], update_fields: &[String]) -> String;

    /// Whether the dialect supports `RETURNING *` on INSERT/UPDATE.
    fn supports_returning(&self) -> bool;

    /// What a multi-row `VALUES` list gives a key the database assigns, for a row with
    /// none: its default, as Ecto writes `DEFAULT` for a missing column.
    fn assigned_key_value(&self) -> &'static str {
        "DEFAULT"
    }

    /// How a read loads aggregates, as ash_sql's `aggregate_strategy`: each relationship's
    /// aggregates in one subquery, laterally joined to each record where the database
    /// can, else grouped by the relationship's key and joined on it.
    fn aggregate_strategy(&self) -> AggregateStrategy;

    /// Render boolean literal for SQL statements.
    fn boolean_literal(&self, val: bool) -> &'static str {
        if val {
            "1"
        } else {
            "0"
        }
    }

    /// Whether table creation uses IF NOT EXISTS.
    fn create_table_if_not_exists(&self) -> bool {
        true
    }

    /// Whether a `CREATE TABLE` may declare a foreign key to a table that doesn't exist
    /// yet. SQLite checks foreign keys only when rows are written; Postgres resolves the
    /// target when the key is created.
    fn allows_forward_references(&self) -> bool {
        false
    }

    /// Render membership check (`IN` / `= ANY(...)`).
    ///
    /// Given an operand expression `op` (e.g. `"tickets"."id"`) and a bound parameter placeholder `param`,
    /// renders the dialect-specific SQL test.
    fn render_in_list(&self, op: &str, param: &str) -> String;

    /// Cast a bound list parameter so its elements compare as the column type.
    fn cast_list_param(&self, _ty: AttrType, placeholder: &str) -> String {
        placeholder.to_string()
    }

    /// The database extension a column type needs, if any.
    fn extension_for_type(&self, _sql_type: &str) -> Option<&'static str> {
        None
    }

    /// How an atomic statement's subquery locks the records it selects: Postgres's
    /// `FOR UPDATE`. SQLite, whose single writer serializes updates, has none.
    fn lock_clause(&self) -> &'static str {
        " FOR UPDATE"
    }

    /// What an atomic update returns of the table it updates, aliased `__ash_t`.
    fn returning_updated(&self) -> &'static str {
        "__ash_t.*"
    }

    /// Functions the data layer's statements call, created with the tables: on
    /// Postgres, the `ash_raise_error` an atomic update raises its errors through.
    fn database_functions(&self) -> &'static [&'static str] {
        &[]
    }

    /// Pattern to bind for a text filter, with wildcards in `needle` escaped.
    fn text_pattern(&self, kind: TextMatch, needle: &str, _case_insensitive: bool) -> String {
        like_pattern(kind, needle)
    }

    /// Render a text filter on `op`. `pattern` is the placeholder bound to [`Self::text_pattern`].
    fn render_text_match(&self, op: &str, pattern: &str, case_insensitive: bool) -> String;

    /// The parameter a `has` filter binds for `value`: a JSON list holding it, which a
    /// JSON list column contains (Postgres's `@>`).
    fn has_param(&self, value: ash_core::Value) -> ash_core::Value {
        ash_core::Value::Array(vec![value])
    }

    /// A list column `op` holding the value bound as `param` ([`Self::has_param`]).
    fn render_has(&self, op: &str, param: &str) -> String {
        format!("({op} @> {param})")
    }

    /// Where an `ORDER BY` term puts nulls, as Ash orders them: last ascending, first
    /// descending. Postgres does so already; a dialect that doesn't says so here.
    fn null_order(&self, _descending: bool) -> &'static str {
        ""
    }
}

/// How a read loads aggregates over a relationship, as ash_sql's `:lateral` and
/// `:grouped` strategies do. Either way, the aggregates over one relationship share one
/// subquery, each with its own filter, so its rows are read once, not once per aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateStrategy {
    /// `LEFT JOIN LATERAL (SELECT … WHERE related.key = record.key) ON TRUE`: computed for
    /// each record the read returns, as AshPostgres does.
    Lateral,
    /// `LEFT JOIN (SELECT key, … GROUP BY key) ON key = record.key`: computed for every key
    /// at once, as AshSqlite does, where there are no lateral joins.
    Grouped,
}

/// Dialect implementation for SQLite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SqliteDialect;

impl SqlDialect for SqliteDialect {
    fn lock_clause(&self) -> &'static str {
        ""
    }

    // SQLite's RETURNING names only the updated table's columns, unqualified.
    fn returning_updated(&self) -> &'static str {
        "*"
    }

    // A list is JSON text: one of its items equals the value.
    fn has_param(&self, value: ash_core::Value) -> ash_core::Value {
        value
    }

    fn render_has(&self, op: &str, param: &str) -> String {
        format!("EXISTS (SELECT 1 FROM json_each({op}) WHERE json_each.value = {param})")
    }

    // SQLite sorts nulls first ascending.
    fn null_order(&self, descending: bool) -> &'static str {
        if descending { " NULLS FIRST" } else { " NULLS LAST" }
    }

    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn allows_forward_references(&self) -> bool {
        true
    }

    fn placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }

    fn column_type(&self, attr: &AttributeDef) -> String {
        match attr.ty {
            AttrType::Integer | AttrType::Boolean => "INTEGER".to_string(),
            AttrType::Decimal => "NUMERIC".to_string(),
            AttrType::Float => "REAL".to_string(),
            AttrType::Binary => "BLOB".to_string(),
            // NOCASE makes =, IN, ORDER BY, and unique indexes ignore ASCII case.
            AttrType::CiString => "TEXT COLLATE NOCASE".to_string(),
            AttrType::Uuid
            | AttrType::String
            | AttrType::Date
            | AttrType::UtcDatetime { .. }
            | AttrType::Inet
            | AttrType::Vector { .. }
            | AttrType::Atom { .. }
            | AttrType::Map
            | AttrType::Embedded(_)
            | AttrType::TypedMap { .. }
            | AttrType::Union { .. }
            | AttrType::Array { .. } => "TEXT".to_string(),
        }
    }

    fn upsert_clause(&self, identity: &IdentityDef, columns: &[&str], update_fields: &[String]) -> String {
        let key_cols = columns
            .iter()
            .map(|k| self.quote_identifier(k))
            .collect::<Vec<_>>()
            .join(", ");
        // A partial unique index only matches a conflict target that repeats its predicate.
        let target = match identity.predicate {
            Some(predicate) => format!("({key_cols}) WHERE {predicate}"),
            None => format!("({key_cols})"),
        };
        if update_fields.is_empty() {
            format!("ON CONFLICT {target} DO NOTHING")
        } else {
            let set_clauses = update_fields
                .iter()
                .map(|f| format!("{} = excluded.{}", self.quote_identifier(f), self.quote_identifier(f)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("ON CONFLICT {target} DO UPDATE SET {set_clauses}")
        }
    }

    fn supports_returning(&self) -> bool {
        false
    }

    /// SQLite takes no `DEFAULT` in `VALUES`; a NULL `INTEGER PRIMARY KEY` is assigned
    /// the next rowid.
    fn assigned_key_value(&self) -> &'static str {
        "NULL"
    }

    /// SQLite has no lateral joins, so aggregates group, as AshSqlite's do.
    fn aggregate_strategy(&self) -> AggregateStrategy {
        AggregateStrategy::Grouped
    }

    fn boolean_literal(&self, val: bool) -> &'static str {
        if val {
            "1"
        } else {
            "0"
        }
    }

    fn render_in_list(&self, op: &str, param: &str) -> String {
        format!("{op} IN (SELECT value FROM json_each({param}))")
    }

    /// `GLOB` is case-sensitive. `LIKE` ignores case, but only for ASCII letters.
    fn text_pattern(&self, kind: TextMatch, needle: &str, case_insensitive: bool) -> String {
        if case_insensitive {
            like_pattern(kind, needle)
        } else {
            glob_pattern(kind, needle)
        }
    }

    fn render_text_match(&self, op: &str, pattern: &str, case_insensitive: bool) -> String {
        if case_insensitive {
            format!("{op} LIKE {pattern} ESCAPE '\\'")
        } else {
            format!("{op} GLOB {pattern}")
        }
    }
}

/// Dialect implementation for PostgreSQL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PostgresDialect;

impl SqlDialect for PostgresDialect {
    fn qualify_table(&self, schema: &str, table: &str) -> Option<String> {
        Some(format!("{schema}.{table}"))
    }

    fn name(&self) -> &'static str {
        "postgres"
    }

    fn placeholder(&self, index: usize) -> String {
        format!("${index}")
    }

    fn cast_expression(&self, ty: AttrType, expression: &str) -> String {
        let column = self.column_type(&AttributeDef::required("", ty));
        format!("CAST({expression} AS {column})")
    }

    fn cast_param(&self, ty: AttrType, placeholder: &str) -> String {
        match ty {
            AttrType::UtcDatetime { .. } => format!("{placeholder}::timestamptz"),
            AttrType::Decimal => format!("{placeholder}::numeric"),
            AttrType::Float => format!("{placeholder}::float8"),
            AttrType::Date => format!("{placeholder}::date"),
            AttrType::CiString => format!("{placeholder}::citext"),
            AttrType::Inet => format!("{placeholder}::inet"),
            AttrType::Vector { .. } => format!("{placeholder}::vector"),
            AttrType::Binary => format!("decode({placeholder}, 'base64')"),
            _ => placeholder.to_string(),
        }
    }

    fn column_type(&self, attr: &AttributeDef) -> String {
        // An integer key the database assigns, as AshPostgres migrates
        // `integer_primary_key` to `bigserial`.
        if attr.primary_key && attr.generated && attr.ty == AttrType::Integer {
            return "BIGSERIAL".to_string();
        }
        match attr.ty {
            AttrType::Uuid => "UUID".to_string(),
            AttrType::String => "TEXT".to_string(),
            AttrType::Atom { .. } => "VARCHAR(255)".to_string(),
            AttrType::Integer => "BIGINT".to_string(),
            AttrType::Boolean => "BOOLEAN".to_string(),
            AttrType::UtcDatetime { .. } => "TIMESTAMPTZ".to_string(),
            AttrType::Decimal => "NUMERIC".to_string(),
            AttrType::Float => "DOUBLE PRECISION".to_string(),
            AttrType::Date => "DATE".to_string(),
            AttrType::CiString => "CITEXT".to_string(),
            AttrType::Binary => "BYTEA".to_string(),
            AttrType::Inet => "INET".to_string(),
            AttrType::Vector { dimensions } => format!("VECTOR({dimensions})"),
            AttrType::Map | AttrType::Array { .. } | AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. } => "JSONB".to_string(),
        }
    }

    fn upsert_clause(&self, identity: &IdentityDef, columns: &[&str], update_fields: &[String]) -> String {
        let key_cols = columns
            .iter()
            .map(|k| self.quote_identifier(k))
            .collect::<Vec<_>>()
            .join(", ");
        // A partial unique index only matches a conflict target that repeats its predicate.
        let target = match identity.predicate {
            Some(predicate) => format!("({key_cols}) WHERE {predicate}"),
            None => format!("({key_cols})"),
        };
        // `DO NOTHING` returns no row for an existing record, so with nothing to update
        // we rewrite a key column to itself and `RETURNING *` still yields the record.
        let fields: Vec<&str> = if update_fields.is_empty() {
            columns.iter().take(1).copied().collect()
        } else {
            update_fields.iter().map(String::as_str).collect()
        };
        if fields.is_empty() {
            format!("ON CONFLICT {target} DO NOTHING")
        } else {
            let set_clauses = fields
                .iter()
                .map(|f| format!("{} = EXCLUDED.{}", self.quote_identifier(f), self.quote_identifier(f)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("ON CONFLICT {target} DO UPDATE SET {set_clauses}")
        }
    }

    fn supports_returning(&self) -> bool {
        true
    }

    fn aggregate_strategy(&self) -> AggregateStrategy {
        AggregateStrategy::Lateral
    }

    fn boolean_literal(&self, val: bool) -> &'static str {
        if val {
            "TRUE"
        } else {
            "FALSE"
        }
    }

    fn render_in_list(&self, op: &str, param: &str) -> String {
        format!("{op} = ANY({param})")
    }

    fn database_functions(&self) -> &'static [&'static str] {
        &[ASH_RAISE_ERROR]
    }

    fn extension_for_type(&self, sql_type: &str) -> Option<&'static str> {
        let upper = sql_type.to_ascii_uppercase();
        if upper.starts_with("CITEXT") {
            Some("citext")
        } else if upper.starts_with("VECTOR") {
            Some("vector")
        } else {
            None
        }
    }

    fn cast_list_param(&self, ty: AttrType, placeholder: &str) -> String {
        match ty {
            AttrType::Boolean => format!("{placeholder}::boolean[]"),
            AttrType::UtcDatetime { .. } => format!("{placeholder}::timestamptz[]"),
            AttrType::Decimal => format!("{placeholder}::numeric[]"),
            AttrType::Float => format!("{placeholder}::float8[]"),
            AttrType::Date => format!("{placeholder}::date[]"),
            AttrType::CiString => format!("{placeholder}::citext[]"),
            _ => placeholder.to_string(),
        }
    }

    /// `LIKE` on a `citext` column already ignores case, so one form covers both.
    fn render_text_match(&self, op: &str, pattern: &str, case_insensitive: bool) -> String {
        if case_insensitive {
            format!("{op} ILIKE {pattern}")
        } else {
            format!("{op} LIKE {pattern}")
        }
    }

    fn binary_literal(&self, encoded: &str) -> String {
        format!("decode('{}', 'base64')", encoded.replace('\'', "''"))
    }
}

/// Raises an error carrying `json_data`, as AshPostgres's function of the same name does:
/// the message is `ash_error: ` and the JSON, which the data layer turns back into the
/// error. An atomic update calls it when one of its conditions holds.
pub const ASH_RAISE_ERROR: &str = "CREATE OR REPLACE FUNCTION ash_raise_error(json_data jsonb) \
RETURNS BOOLEAN AS $$ \
BEGIN \
    RAISE EXCEPTION 'ash_error: %', json_data::text; \
    RETURN NULL; \
END; \
$$ LANGUAGE plpgsql STABLE SET search_path = '';";
