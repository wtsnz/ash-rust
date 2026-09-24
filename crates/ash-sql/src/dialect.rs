use ash_core::{AttrType, AttributeDef, IdentityDef};

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
            AttrType::Uuid
            | AttrType::String
            | AttrType::Date
            | AttrType::UtcDatetime
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
        if update_fields.is_empty() {
            format!("ON CONFLICT ({key_cols}) DO NOTHING")
        } else {
            let set_clauses = update_fields
                .iter()
                .map(|f| format!("{} = excluded.{}", self.quote_identifier(f), self.quote_identifier(f)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("ON CONFLICT ({key_cols}) DO UPDATE SET {set_clauses}")
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

    fn cast_param(&self, ty: AttrType, placeholder: &str) -> String {
        match ty {
            AttrType::UtcDatetime => format!("{placeholder}::timestamptz"),
            AttrType::Decimal => format!("{placeholder}::numeric"),
            AttrType::Float => format!("{placeholder}::float8"),
            AttrType::Date => format!("{placeholder}::date"),
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
            AttrType::Binary => "BYTEA".to_string(),
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
        if update_fields.is_empty() {
            format!("ON CONFLICT ({key_cols}) DO NOTHING")
        } else {
            let set_clauses = update_fields
                .iter()
                .map(|f| format!("{} = EXCLUDED.{}", self.quote_identifier(f), self.quote_identifier(f)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("ON CONFLICT ({key_cols}) DO UPDATE SET {set_clauses}")
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

    fn binary_literal(&self, encoded: &str) -> String {
        format!("decode('{}', 'base64')", encoded.replace('\'', "''"))
    }
}
