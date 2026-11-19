//! SQL as the console reads it, apart from drawing it: each engine's dialect,
//! highlighting, where statements start and end, completion and the snippets
//! the explorer writes; and the documents of MongoDB and Cosmos DB, which are
//! highlighted but not SQL. Pure functions, so they are tested without a window.

pub mod completion;
pub mod document;
pub mod highlight;
pub mod snippets;
pub mod statements;
pub mod structure;

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

/// What an editor's text is, for highlighting and completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Syntax {
    /// Text, neither highlighted nor completed.
    #[default]
    Plain,
    /// SQL in a dialect: highlighted and completed.
    Sql(Dialect),
    /// JSON: Cosmos DB's documents.
    Json,
    /// mongosh's JavaScript: MongoDB's documents and console.
    Mongosh,
}

impl Syntax {
    /// What `engine`'s console is written in: mongosh for MongoDB, else its SQL.
    pub fn console(engine: Engine) -> Self {
        match engine {
            Engine::Mongodb => Self::Mongosh,
            engine => Dialect::of(engine).map_or(Self::Plain, Self::Sql),
        }
    }

    /// What `engine`'s documents are written in: mongosh's literals, or JSON.
    pub fn documents(engine: Engine) -> Self {
        if engine == Engine::Mongodb { Self::Mongosh } else { Self::Json }
    }

    /// The SQL dialect, when it is SQL.
    pub fn dialect(self) -> Option<Dialect> {
        match self {
            Self::Sql(dialect) => Some(dialect),
            _ => None,
        }
    }

    /// What starts a line comment (⌘/ toggles it): `--` in SQL, `//` in
    /// mongosh; JSON and plain text have none, as in CodeMirror's languages.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Self::Sql(_) => Some("--"),
            Self::Mongosh => Some("//"),
            Self::Json | Self::Plain => None,
        }
    }

    /// The brackets and quotes typing one of closes, as CodeMirror's
    /// `closeBrackets` takes them from each language (its defaults for text).
    pub fn closing_brackets(self) -> &'static [char] {
        match self {
            Self::Sql(_) | Self::Mongosh => &['(', '[', '{', '\'', '"', '`'],
            Self::Json => &['[', '{', '"'],
            Self::Plain => &['(', '[', '{', '\'', '"'],
        }
    }
}

impl From<Dialect> for Syntax {
    fn from(dialect: Dialect) -> Self {
        Self::Sql(dialect)
    }
}

impl From<Option<Dialect>> for Syntax {
    fn from(dialect: Option<Dialect>) -> Self {
        dialect.map_or(Self::Plain, Self::Sql)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_each_engine_s_console_and_documents_in_its_own_syntax() {
        assert_eq!(Syntax::console(Engine::Mongodb), Syntax::Mongosh);
        assert_eq!(Syntax::console(Engine::Cosmosdb), Syntax::Sql(Dialect::Cosmos));
        assert_eq!(Syntax::console(Engine::Mariadb), Syntax::Sql(Dialect::Mysql));
        assert_eq!(Syntax::console(Engine::Redis), Syntax::Plain);
        assert_eq!(Syntax::documents(Engine::Mongodb), Syntax::Mongosh);
        assert_eq!(Syntax::documents(Engine::Cosmosdb), Syntax::Json);
    }

    #[test]
    fn comments_and_closes_brackets_as_each_language_does() {
        assert_eq!(Syntax::Sql(Dialect::Mysql).line_comment(), Some("--"));
        assert_eq!(Syntax::Mongosh.line_comment(), Some("//"));
        assert_eq!(Syntax::Json.line_comment(), None, "JSON has no comments");
        assert_eq!(Syntax::Plain.line_comment(), None);
        assert!(Syntax::Sql(Dialect::Postgres).closing_brackets().contains(&'`'));
        assert!(!Syntax::Json.closing_brackets().contains(&'('), "JSON closes only [, {{ and \"");
        assert!(!Syntax::Plain.closing_brackets().contains(&'`'));
    }
}
