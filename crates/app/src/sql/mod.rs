//! SQL as the console reads it, apart from drawing it: each engine's dialect,
//! highlighting, where statements start and end, completion and the snippets
//! the explorer writes. Pure functions, so they are tested without a window.

pub mod completion;
pub mod highlight;
pub mod snippets;
pub mod statements;

use dbforge_core::connections::model::Engine;

/// A query language with its own keywords, quoting and comments. MongoDB's
/// mongosh scripts and Redis commands are not SQL and have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Postgres,
    /// MySQL and MariaDB.
    Mysql,
    Sqlite,
    Sqlserver,
    /// Cosmos DB's SQL over JSON items: standard SQL.
    Cosmos,
}

impl Dialect {
    /// The dialect `engine` is queried in; `None` for engines without SQL.
    pub fn of(engine: Engine) -> Option<Self> {
        match engine {
            Engine::Postgres => Some(Self::Postgres),
            Engine::Mysql | Engine::Mariadb => Some(Self::Mysql),
            Engine::Sqlite => Some(Self::Sqlite),
            Engine::Sqlserver => Some(Self::Sqlserver),
            Engine::Cosmosdb => Some(Self::Cosmos),
            Engine::Mongodb | Engine::Redis => None,
        }
    }

    /// The schema a database's tables are named without (`public`, `dbo`,
    /// `main`); `None` where the database is the only schema (MySQL).
    pub fn default_schema(self) -> Option<&'static str> {
        match self {
            Self::Postgres => Some("public"),
            Self::Sqlserver => Some("dbo"),
            Self::Sqlite => Some("main"),
            Self::Mysql | Self::Cosmos => None,
        }
    }

    /// `#` starts a comment, as `--` does everywhere.
    fn hash_comments(self) -> bool {
        self == Self::Mysql
    }

    /// The characters a quoted identifier opens and closes with.
    fn identifier_quotes(self) -> &'static [(char, char)] {
        match self {
            Self::Mysql => &[('`', '`')],
            Self::Sqlserver => &[('[', ']'), ('"', '"')],
            Self::Sqlite => &[('"', '"'), ('`', '`'), ('[', ']')],
            Self::Postgres | Self::Cosmos => &[('"', '"')],
        }
    }

    /// `$$ … $$` and `$tag$ … $tag$` strings, as in function bodies.
    fn dollar_strings(self) -> bool {
        self == Self::Postgres
    }
}
