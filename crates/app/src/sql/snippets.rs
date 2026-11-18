//! SQL the explorer writes for a table: its name as a console on its
//! database names it, and the query a "New query" on it starts with
//! (mongosh's for a MongoDB collection).

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::TableRef;

use super::Dialect;

/// Rows a "New query" from the explorer asks for.
const PREVIEW_ROWS: u32 = 100;

/// The table a tree node's path names: `[table]` (SQLite), `[database, table]`
/// (a database that is its own schema) or `[database, schema, table]`.
pub fn table_ref(path: &[String]) -> Option<TableRef> {
    let owned = |name: &String| name.clone();
    match path {
        [name] => Some(TableRef { database: "main".into(), schema: "main".into(), name: owned(name) }),
        [database, name] => Some(TableRef { database: owned(database), schema: owned(database), name: owned(name) }),
        [database, schema, name] => {
            Some(TableRef { database: owned(database), schema: owned(schema), name: owned(name) })
        }
        _ => None,
    }
}

/// A name quoted as the dialect quotes it, its quote doubled inside.
pub fn quote(dialect: Dialect, name: &str) -> String {
    match dialect {
        Dialect::Mysql => format!("`{}`", name.replace('`', "``")),
        Dialect::Sqlserver => format!("[{}]", name.replace(']', "]]")),
        Dialect::Cosmos => {
            let plain = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if plain { format!("c.{name}") } else { format!("c[{}]", serde_json::Value::from(name)) }
        }
        Dialect::Postgres | Dialect::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// A table as SQL names it from a console opened on its database. Schemaless
/// engines (MySQL) and SQLite's `main` need no schema.
pub fn qualified_name(dialect: Dialect, table: &TableRef) -> String {
    if dialect == Dialect::Mysql || (dialect == Dialect::Sqlite && table.schema == "main") {
        return quote(dialect, &table.name);
    }
    format!("{}.{}", quote(dialect, &table.schema), quote(dialect, &table.name))
}

/// The first rows of a table, as `engine`'s console writes it; `None` for
/// an engine without a console (Redis).
pub fn preview_query(engine: Engine, table: &TableRef) -> Option<String> {
    if engine == Engine::Mongodb {
        return Some(format!("db.getCollection({}).find({{}}).limit({PREVIEW_ROWS})", collection(table)));
    }
    Some(sql_preview(Dialect::of(engine)?, table))
}

/// A collection's count in mongosh.
pub fn count_query(table: &TableRef) -> String {
    format!("db.getCollection({}).countDocuments({{}})", collection(table))
}

/// A collection's name as mongosh writes it: a JSON string.
fn collection(table: &TableRef) -> String {
    serde_json::Value::from(table.name.as_str()).to_string()
}

fn sql_preview(dialect: Dialect, table: &TableRef) -> String {
    match dialect {
        // The console runs on the container it was opened with.
        Dialect::Cosmos => format!("SELECT TOP {PREVIEW_ROWS} * FROM c"),
        Dialect::Sqlserver => format!("SELECT TOP {PREVIEW_ROWS} * FROM {};", qualified_name(dialect, table)),
        _ => format!("SELECT * FROM {} LIMIT {PREVIEW_ROWS};", qualified_name(dialect, table)),
    }
}

/// Every column, one per line, ready to trim.
pub fn select_statement(dialect: Dialect, table: &TableRef, columns: &[String]) -> String {
    let list: Vec<String> = columns.iter().map(|column| format!("  {}", quote(dialect, column))).collect();
    format!("SELECT\n{}\nFROM {};", list.join(",\n"), qualified_name(dialect, table))
}

/// An INSERT naming every column, with NULL in each value to fill in.
pub fn insert_statement(dialect: Dialect, table: &TableRef, columns: &[String]) -> String {
    let names: Vec<String> = columns.iter().map(|column| quote(dialect, column)).collect();
    let values = vec!["NULL"; columns.len()];
    format!("INSERT INTO {} ({})\nVALUES ({});", qualified_name(dialect, table), names.join(", "), values.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(database: &str, schema: &str, name: &str) -> TableRef {
        TableRef { database: database.into(), schema: schema.into(), name: name.into() }
    }

    #[test]
    fn names_a_table_the_way_each_dialect_quotes_it() {
        let orders = table("shop", "sales", "order \"items\"");
        assert_eq!(qualified_name(Dialect::Postgres, &orders), "\"sales\".\"order \"\"items\"\"\"");
        assert_eq!(qualified_name(Dialect::Sqlserver, &table("shop", "sales", "a]b")), "[sales].[a]]b]");
        assert_eq!(qualified_name(Dialect::Mysql, &table("shop", "shop", "a`b")), "`a``b`");
        assert_eq!(qualified_name(Dialect::Sqlite, &table("main", "main", "users")), "\"users\"");
    }

    #[test]
    fn asks_each_engine_for_its_first_hundred_rows() {
        let users = table("shop", "public", "users");
        let preview = |engine, table: &TableRef| preview_query(engine, table).unwrap();
        assert_eq!(preview(Engine::Postgres, &users), "SELECT * FROM \"public\".\"users\" LIMIT 100;");
        assert_eq!(preview(Engine::Sqlserver, &table("shop", "dbo", "users")), "SELECT TOP 100 * FROM [dbo].[users];");
        assert_eq!(
            preview(Engine::Mongodb, &table("shop", "shop", "my-orders")),
            "db.getCollection(\"my-orders\").find({}).limit(100)"
        );
        assert_eq!(preview(Engine::Cosmosdb, &users), "SELECT TOP 100 * FROM c");
        assert_eq!(preview_query(Engine::Redis, &users), None);
    }

    #[test]
    fn counts_a_collection_in_mongosh() {
        assert_eq!(count_query(&table("shop", "shop", "a\"b")), "db.getCollection(\"a\\\"b\").countDocuments({})");
    }

    #[test]
    fn writes_select_and_insert_with_every_column() {
        let users = table("shop", "public", "users");
        let columns = ["id".to_owned(), "full name".to_owned()];
        assert_eq!(
            select_statement(Dialect::Postgres, &users, &columns),
            "SELECT\n  \"id\",\n  \"full name\"\nFROM \"public\".\"users\";"
        );
        let columns = ["id".to_owned(), "name".to_owned()];
        assert_eq!(
            insert_statement(Dialect::Mysql, &table("shop", "shop", "users"), &columns),
            "INSERT INTO `users` (`id`, `name`)\nVALUES (NULL, NULL);"
        );
    }

    #[test]
    fn reads_a_table_from_its_tree_path() {
        let path = |parts: &[&str]| parts.iter().map(|part| part.to_string()).collect::<Vec<_>>();
        let sqlite = table_ref(&path(&["users"])).unwrap();
        assert_eq!((sqlite.database.as_str(), sqlite.schema.as_str()), ("main", "main"));
        let mysql = table_ref(&path(&["shop", "users"])).unwrap();
        assert_eq!((mysql.schema.as_str(), mysql.name.as_str()), ("shop", "users"));
        assert_eq!(table_ref(&path(&["shop", "public", "users"])).unwrap().schema, "public");
        assert!(table_ref(&[]).is_none());
    }
}
