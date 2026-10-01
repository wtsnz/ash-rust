use ash_core::{AttrType, AttributeDef, IdentityDef};

/// Where a text filter looks for its needle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMatch {
    Contains,
    StartsWith,
    EndsWith,
}

/// `LIKE` pattern with `\`, `%`, and `_` in `needle` escaped by a backslash.
pub fn like_pattern(kind: TextMatch, needle: &str) -> String {
    let mut escaped = String::with_capacity(needle.len());
    for ch in needle.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    wrap_pattern(kind, &escaped, "%")
}

/// SQLite `GLOB` pattern with `*`, `?`, and `[` in `needle` matched literally.
pub fn glob_pattern(kind: TextMatch, needle: &str) -> String {
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
    fn upsert_clause(&self, identity: &IdentityDef, update_fields: &[String]) -> String;

    /// Whether the dialect supports `RETURNING *` on INSERT/UPDATE.
    fn supports_returning(&self) -> bool;

    /// Render lateral join syntax for aggregates/relationships.
    fn render_lateral_join(&self, subquery: &str, alias: &str) -> String;

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

    /// Pattern to bind for a text filter, with wildcards in `needle` escaped.
    fn text_pattern(&self, kind: TextMatch, needle: &str, _case_insensitive: bool) -> String {
        like_pattern(kind, needle)
    }

    /// Render a text filter on `op`. `pattern` is the placeholder bound to [`Self::text_pattern`].
    fn render_text_match(&self, op: &str, pattern: &str, case_insensitive: bool) -> String;
}

/// Dialect implementation for SQLite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SqliteDialect;

impl SqlDialect for SqliteDialect {
    fn name(&self) -> &'static str {
        "sqlite"
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
            | AttrType::UtcDatetime
            | AttrType::Inet
            | AttrType::Vector { .. }
            | AttrType::Atom { .. }
            | AttrType::Map
            | AttrType::Array => "TEXT".to_string(),
        }
    }

    fn upsert_clause(&self, identity: &IdentityDef, update_fields: &[String]) -> String {
        let key_cols = identity
            .keys
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

    fn render_lateral_join(&self, subquery: &str, alias: &str) -> String {
        format!("(SELECT * FROM ({subquery})) AS {}", self.quote_identifier(alias))
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
            AttrType::UtcDatetime => format!("{placeholder}::timestamptz"),
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
        match attr.ty {
            AttrType::Uuid => "UUID".to_string(),
            AttrType::String => "TEXT".to_string(),
            AttrType::Atom { .. } => "VARCHAR(255)".to_string(),
            AttrType::Integer => "BIGINT".to_string(),
            AttrType::Boolean => "BOOLEAN".to_string(),
            AttrType::UtcDatetime => "TIMESTAMPTZ".to_string(),
            AttrType::Decimal => "NUMERIC".to_string(),
            AttrType::Float => "DOUBLE PRECISION".to_string(),
            AttrType::Date => "DATE".to_string(),
            AttrType::CiString => "CITEXT".to_string(),
            AttrType::Binary => "BYTEA".to_string(),
            AttrType::Inet => "INET".to_string(),
            AttrType::Vector { dimensions } => format!("VECTOR({dimensions})"),
            AttrType::Map | AttrType::Array => "JSONB".to_string(),
        }
    }

    fn upsert_clause(&self, identity: &IdentityDef, update_fields: &[String]) -> String {
        let key_cols = identity
            .keys
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
            identity.keys.iter().take(1).copied().collect()
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

    fn render_lateral_join(&self, subquery: &str, alias: &str) -> String {
        format!("LEFT JOIN LATERAL ({subquery}) AS {} ON true", self.quote_identifier(alias))
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
            AttrType::UtcDatetime => format!("{placeholder}::timestamptz[]"),
            AttrType::Decimal => format!("{placeholder}::numeric[]"),
            AttrType::Float => format!("{placeholder}::float8[]"),
            AttrType::Date => format!("{placeholder}::date[]"),
            AttrType::CiString => format!("{placeholder}::citext[]"),
            _ => placeholder.to_string(),
        }
    }

    /// `LIKE` on a `citext` column already ignores case, so one form covers both.
    fn render_text_match(&self, op: &str, pattern: &str, _case_insensitive: bool) -> String {
        format!("{op} LIKE {pattern}")
    }

    fn binary_literal(&self, encoded: &str) -> String {
        format!("decode('{}', 'base64')", encoded.replace('\'', "''"))
    }
}
