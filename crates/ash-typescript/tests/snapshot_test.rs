use ash_sql::{ColumnSnapshot, ReferenceSnapshot, TableSnapshot};
use ash_typescript::{generate_from_snapshots, TypeScriptConfig};

#[test]
fn test_generate_from_snapshots() {
    let user_snapshot = TableSnapshot {
        format_version: 1,
        table: "users".to_string(),
        schema: None,
        columns: vec![
            ColumnSnapshot {
                name: "id".to_string(),
                sql_type: "UUID".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
            },
            ColumnSnapshot {
                name: "email".to_string(),
                sql_type: "VARCHAR".to_string(),
                nullable: false,
                default: None,
                is_primary_key: false,
            },
        ],
        primary_key: vec!["id".to_string()],
        identities: vec![],
        indexes: vec![],
        checks: vec![],
        statements: vec![],
        references: vec![],
    };

    let ticket_snapshot = TableSnapshot {
        format_version: 1,
        table: "tickets".to_string(),
        schema: None,
        columns: vec![
            ColumnSnapshot {
                name: "id".to_string(),
                sql_type: "UUID".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
            },
            ColumnSnapshot {
                name: "title".to_string(),
                sql_type: "TEXT".to_string(),
                nullable: false,
                default: None,
                is_primary_key: false,
            },
            ColumnSnapshot {
                name: "author_id".to_string(),
                sql_type: "UUID".to_string(),
                nullable: true,
                default: None,
                is_primary_key: false,
            },
        ],
        primary_key: vec!["id".to_string()],
        identities: vec![],
        indexes: vec![],
        checks: vec![],
        statements: vec![],
        references: vec![ReferenceSnapshot {
            name: "fk_tickets_author".to_string(),
            column: "author_id".to_string(),
            columns: vec!["author_id".to_string()],
            target_table: "users".to_string(),
            target_column: "id".to_string(),
            target_columns: vec!["id".to_string()],
            on_delete: "SET NULL".to_string(),
            on_update: "NO ACTION".to_string(),
        }],
    };

    let config = TypeScriptConfig::new()
        .with_client_name("AshClient")
        .with_graphql_endpoint("/graphql");

    let ts = generate_from_snapshots(&[user_snapshot, ticket_snapshot], &config).unwrap();

    assert!(ts.contains("export interface User {"));
    assert!(ts.contains("export interface Ticket {"));
    assert!(ts.contains("author?: User | null;"));
    assert!(ts.contains("export const UserSchema = z.object({"));
    assert!(ts.contains("export const TicketSchema = z.object({"));
    assert!(ts.contains("export class TicketQueryBuilder {"));
    assert!(ts.contains("export class AshClient {"));
    assert!(ts.contains("public readonly ticket: TicketClient;"));
    assert!(ts.contains("public readonly user: UserClient;"));
}

#[test]
fn test_json_columns_have_no_filter_type() {
    use ash_typescript::snapshot::sql_type_to_ts_and_zod;

    assert_eq!(sql_type_to_ts_and_zod("UUID", false).1, Some("UuidFilter"));
    assert_eq!(sql_type_to_ts_and_zod("JSONB", true).1, None);
    let decimal = sql_type_to_ts_and_zod("NUMERIC", false);
    assert_eq!((decimal.0, decimal.1), ("string", Some("StringFilter")));
}

#[test]
fn test_only_ci_string_columns_get_text_filters() {
    use ash_typescript::snapshot::sql_type_to_ts_and_zod;

    assert_eq!(sql_type_to_ts_and_zod("CITEXT", false).1, Some("TextFilter"));
    assert_eq!(sql_type_to_ts_and_zod("TEXT COLLATE NOCASE", false).1, Some("TextFilter"));
    // SQLite keeps dates, decimals and enums in TEXT, and Postgres enums in VARCHAR.
    assert_eq!(sql_type_to_ts_and_zod("TEXT", false).1, Some("StringFilter"));
    assert_eq!(sql_type_to_ts_and_zod("VARCHAR(255)", false).1, Some("StringFilter"));
}
