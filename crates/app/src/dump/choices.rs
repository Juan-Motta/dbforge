//! What a database's dialogs let one choose for its engine, and the core's
//! options those choices make: kept apart from the views so each engine's
//! defaults and requests are checked without a window.

use dbforge_core::dump::{
    Content, DumpEngine, DumpOptions, MongodbOptions, MysqlOptions, PostgresOptions, SqliteFormat, SqliteOptions,
};

/// An export's choices for its engine.
#[derive(Debug, Clone)]
pub enum ExportChoices {
    Postgres(PostgresOptions),
    /// MySQL and MariaDB.
    Mysql(MysqlOptions),
    Mongodb(Collections),
    Sqlite(SqliteFormat),
}

/// Which of a MongoDB database's collections an export holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collections {
    /// The whole database; when off, only `chosen`.
    pub whole: bool,
    /// In the order they were ticked.
    pub chosen: Vec<String>,
}

impl Collections {
    /// Ticks `name` on or off.
    pub fn toggle(&mut self, name: &str, on: bool) {
        self.chosen.retain(|chosen| chosen != name);
        if on {
            self.chosen.push(name.to_owned());
        }
    }
}

impl ExportChoices {
    /// What a new export of `engine` starts with: everything, read consistently.
    pub fn new(engine: DumpEngine) -> Self {
        match engine {
            DumpEngine::Postgres => {
                Self::Postgres(PostgresOptions { content: Content::All, clean: false, keep_owners: false })
            }
            DumpEngine::Mysql => Self::Mysql(MysqlOptions {
                content: Content::All,
                add_drop_table: false,
                single_transaction: true,
                routines: true,
            }),
            DumpEngine::Mongodb => Self::Mongodb(Collections { whole: true, chosen: Vec::new() }),
            DumpEngine::Sqlite => Self::Sqlite(SqliteFormat::Script),
        }
    }

    /// Whether the export can run: a MongoDB export of chosen collections needs one.
    pub fn ready(&self) -> bool {
        match self {
            Self::Mongodb(collections) => collections.whole || !collections.chosen.is_empty(),
            _ => true,
        }
    }

    /// The options the core exports with.
    pub fn options(&self) -> DumpOptions {
        match self {
            Self::Postgres(options) => DumpOptions::Postgres(options.clone()),
            Self::Mysql(options) => DumpOptions::Mysql(options.clone()),
            Self::Mongodb(collections) => DumpOptions::Mongodb(MongodbOptions {
                collections: if collections.whole { Vec::new() } else { collections.chosen.clone() },
            }),
            Self::Sqlite(format) => DumpOptions::Sqlite(SqliteOptions { format: *format }),
        }
    }

    /// The extension of the file it writes, dot included.
    pub fn extension(&self) -> &'static str {
        self.options().extension()
    }
}

/// `name` with its `from` extension turned into `to`; as typed when it doesn't end with `from`.
pub fn swap_extension(name: &str, from: &str, to: &str) -> String {
    match name.strip_suffix(from) {
        Some(stem) if from != to => format!("{stem}{to}"),
        _ => name.to_owned(),
    }
}

/// The name a SQLite connection's export and dialogs go by: its file's, without the extension.
pub fn sqlite_name(file: &str) -> String {
    let name = super::file_name_of(file);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

/// Where a SQLite restore writes: the connection's own file, or a new one once it is picked.
pub fn sqlite_target(own_file: &str, into_new: bool, new_file: Option<&str>) -> Option<String> {
    if into_new { new_file.map(str::to_owned) } else { Some(own_file.to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_engine_starts_with_its_own_options_and_extension() {
        let postgres = ExportChoices::new(DumpEngine::Postgres);
        assert!(matches!(
            postgres.options(),
            DumpOptions::Postgres(PostgresOptions { content: Content::All, clean: false, keep_owners: false })
        ));
        assert_eq!(postgres.extension(), ".sql");

        let mysql = ExportChoices::new(DumpEngine::Mysql);
        assert!(matches!(
            mysql.options(),
            DumpOptions::Mysql(MysqlOptions {
                content: Content::All,
                add_drop_table: false,
                single_transaction: true,
                routines: true
            })
        ));
        assert_eq!(mysql.extension(), ".sql");

        let mongodb = ExportChoices::new(DumpEngine::Mongodb);
        assert!(
            matches!(mongodb.options(), DumpOptions::Mongodb(MongodbOptions { collections }) if collections.is_empty())
        );
        assert_eq!(mongodb.extension(), ".archive.gz");

        let sqlite = ExportChoices::new(DumpEngine::Sqlite);
        assert!(matches!(sqlite.options(), DumpOptions::Sqlite(SqliteOptions { format: SqliteFormat::Script })));
        assert_eq!(sqlite.extension(), ".sql");
        assert_eq!(ExportChoices::Sqlite(SqliteFormat::File).extension(), ".db");
        assert!([postgres, mysql, sqlite].iter().all(ExportChoices::ready));
    }

    #[test]
    fn a_mongodb_export_of_chosen_collections_names_them_and_needs_one() {
        let mut collections = Collections { whole: false, chosen: Vec::new() };
        assert!(!ExportChoices::Mongodb(collections.clone()).ready(), "no collection chosen");
        collections.toggle("orders", true);
        collections.toggle("users", true);
        collections.toggle("orders", false);
        collections.toggle("items", true);
        let choices = ExportChoices::Mongodb(collections.clone());
        assert!(choices.ready());
        assert!(
            matches!(choices.options(), DumpOptions::Mongodb(MongodbOptions { collections }) if collections == ["users", "items"])
        );
        collections.whole = true;
        assert!(
            matches!(ExportChoices::Mongodb(collections).options(), DumpOptions::Mongodb(MongodbOptions { collections }) if collections.is_empty()),
            "the whole database names none"
        );
    }

    #[test]
    fn swaps_only_the_extension_the_name_ends_with() {
        assert_eq!(swap_extension("shop-2026-11-10-1432.sql", ".sql", ".db"), "shop-2026-11-10-1432.db");
        assert_eq!(swap_extension("mine.txt", ".sql", ".db"), "mine.txt", "a typed name stays");
        assert_eq!(swap_extension("shop.sql", ".sql", ".sql"), "shop.sql");
    }

    #[test]
    fn a_sqlite_database_goes_by_its_file_and_restores_into_it_or_a_new_one() {
        assert_eq!(sqlite_name("/data/shop.db"), "shop");
        assert_eq!(sqlite_name("C:\\data\\shop.sqlite3"), "shop");
        assert_eq!(sqlite_name("/data/.hidden"), ".hidden");
        assert_eq!(sqlite_target("/data/shop.db", false, Some("/x/new.db")).as_deref(), Some("/data/shop.db"));
        assert_eq!(sqlite_target("/data/shop.db", true, None), None, "a new file must be picked first");
        assert_eq!(sqlite_target("/data/shop.db", true, Some("/x/new.db")).as_deref(), Some("/x/new.db"));
    }
}
