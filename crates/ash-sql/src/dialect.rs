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
            AttrType::Uuid
            | AttrType::String
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

    fn column_type(&self, attr: &AttributeDef) -> String {
        match attr.ty {
            AttrType::Uuid => "UUID".to_string(),
            AttrType::String => "TEXT".to_string(),
            AttrType::Atom { .. } => "VARCHAR(255)".to_string(),
            AttrType::Integer => "BIGINT".to_string(),
            AttrType::Boolean => "BOOLEAN".to_string(),
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
}
