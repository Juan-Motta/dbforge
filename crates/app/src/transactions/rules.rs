//! The rules of a console's transaction, apart from drawing it: the mode a
//! console starts in, the statements Manual refuses, how long one has been
//! open, when it warns of being idle, and what its capsule says. Pure
//! functions, so they are tested without a window.

use std::time::Duration;

use dbforge_core::query::{OpenTransaction, TransactionMode};

use crate::i18n::TransactionMessages;
use crate::sql::Dialect;
use crate::sql::highlight::{Kind, LineState, tokenize};
use crate::sql::statements::statements;

/// How long a transaction stays idle before its capsule and a notice warn
/// that it holds locks; the notice repeats every time this much more passes.
pub const IDLE_WARNING: Duration = Duration::from_secs(10 * 60);

/// The mode a console starts in: Manual on a connection whose tag asks for
/// confirmation (`confirming_tag`), so its changes wait for Commit.
pub fn default_mode(confirming_tag: Option<&str>) -> TransactionMode {
    match confirming_tag {
        Some(_) => TransactionMode::Manual,
        None => TransactionMode::Auto,
    }
}

/// Why Manual refuses a statement, with its leading keywords.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// It starts or ends a transaction itself (`COMMIT`, `START TRANSACTION`):
    /// the console would no longer know whether one is open.
    Control(String),
    /// MySQL and MariaDB commit the open transaction before running it
    /// (`CREATE`, `ALTER`, `LOCK`…): Rollback would no longer undo what came before.
    ImplicitCommit(String),
}

/// The first statement of `sql` that Manual refuses, and why; Manual runs
/// none of the script then. Comments before a statement are skipped; strings
/// and quoted names never count.
pub fn refusal(sql: &str, dialect: Dialect) -> Option<Refusal> {
    statements(sql, dialect).into_iter().find_map(|range| {
        let words = leading_words(&sql[range], dialect);
        controls(&words, dialect)
            .map(Refusal::Control)
            .or_else(|| commits_implicitly(&words, dialect).map(Refusal::ImplicitCommit))
    })
}

/// The first four words of `statement`, upper-cased, after its comments.
fn leading_words(statement: &str, dialect: Dialect) -> Vec<String> {
    let mut state = LineState::Normal;
    let mut offset = 0;
    let mut start = None;
    'lines: for line in statement.split('\n') {
        let (spans, next) = tokenize(line, state, dialect);
        for span in spans.iter().filter(|span| span.kind != Kind::Comment) {
            let text = &line[span.range.clone()];
            let blank = text.len() - text.trim_start().len();
            if blank < text.len() {
                start = Some(offset + span.range.start + blank);
                break 'lines;
            }
        }
        state = next;
        offset += line.len() + 1;
    }
    let Some(start) = start else { return Vec::new() };
    statement[start..]
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .take(4)
        .map(str::to_uppercase)
        .collect()
}

/// Whether a statement starting with `words` starts or ends a transaction in
/// `dialect`: the keywords that say so.
fn controls(words: &[String], dialect: Dialect) -> Option<String> {
    let first = words.first()?.as_str();
    let second = words.get(1).map(String::as_str);
    let both = || format!("{first} {}", second.unwrap_or_default());
    match (first, dialect) {
        (_, Dialect::Cosmos) => None,
        // `ROLLBACK TO` a savepoint keeps the transaction open.
        ("ROLLBACK", _) if second == Some("TO") => None,
        ("COMMIT" | "ROLLBACK", _) => Some(first.to_owned()),
        // T-SQL's `BEGIN … END` and `BEGIN TRY` are blocks, not transactions.
        ("BEGIN", Dialect::Sqlserver) => matches!(second, Some("TRAN" | "TRANSACTION" | "DISTRIBUTED")).then(both),
        ("BEGIN", _) => Some(first.to_owned()),
        ("START", _) if second == Some("TRANSACTION") => Some(both()),
        // `END` commits on PostgreSQL and SQLite; elsewhere it closes a block.
        ("END", Dialect::Postgres | Dialect::Sqlite) => Some(first.to_owned()),
        ("ABORT", Dialect::Postgres) => Some(first.to_owned()),
        _ => None,
    }
}

/// Whether a statement starting with `words` commits the open transaction
/// before it runs, on MySQL and MariaDB: the keyword that does. Temporary
/// tables are the exception; other dialects keep DDL in the transaction.
fn commits_implicitly(words: &[String], dialect: Dialect) -> Option<String> {
    if dialect != Dialect::Mysql {
        return None;
    }
    let first = words.first()?.as_str();
    let rest = &words[1..];
    match first {
        "CREATE" | "DROP" if rest.iter().any(|word| word == "TEMPORARY") => None,
        "CREATE" | "ALTER" | "DROP" | "RENAME" | "TRUNCATE" | "GRANT" | "REVOKE" | "LOCK" | "UNLOCK" | "ANALYZE"
        | "OPTIMIZE" | "REPAIR" | "CHECK" | "FLUSH" | "INSTALL" | "UNINSTALL" => Some(first.to_owned()),
        // `SET autocommit`, `SET @@autocommit`, `SET SESSION autocommit`.
        "SET" if rest.iter().take(2).any(|word| word == "AUTOCOMMIT") => Some("SET AUTOCOMMIT".to_owned()),
        _ => None,
    }
}

/// How long `ms` is, as a clock: `3:42`, `10:12`, `1:02:05` past an hour.
pub fn clock(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// How many idle warnings are due after `idle_ms` without a run: one at
/// 10 minutes, another at 20, at 30…
pub fn idle_warnings(idle_ms: i64) -> u32 {
    (idle_ms.max(0) / IDLE_WARNING.as_millis() as i64) as u32
}

/// The minutes the `count`th idle warning stands for.
pub fn idle_minutes(count: u32) -> u64 {
    u64::from(count) * IDLE_WARNING.as_secs() / 60
}

/// How the capsule looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsuleTone {
    Open,
    /// Idle long enough to warn that it holds locks.
    Idle,
    /// A statement failed in it: only Rollback works.
    Failed,
}

/// What the capsule says: its title and, after it, the details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capsule {
    pub tone: CapsuleTone,
    pub title: String,
    pub detail: String,
}

/// The capsule of `transaction` at `now_ms`: "Transaction open" then its
/// changes, rows and how long it has been open; a failed one says only
/// Rollback works; an idle one warns it holds locks.
pub fn capsule(m: &TransactionMessages, transaction: &OpenTransaction, now_ms: i64) -> Capsule {
    let counts = format!("{} · {}", (m.changes)(transaction.changes), (m.rows)(transaction.rows));
    let open_for = clock(now_ms - transaction.started_at_ms);
    if transaction.aborted {
        return Capsule { tone: CapsuleTone::Failed, title: m.failed.to_owned(), detail: m.only_rollback.to_owned() };
    }
    let warnings = idle_warnings(now_ms - transaction.last_activity_ms);
    let (tone, title) = match warnings {
        0 => (CapsuleTone::Open, m.open.to_owned()),
        count => (CapsuleTone::Idle, (m.idle)(idle_minutes(count))),
    };
    Capsule { tone, title, detail: format!("{counts} · {open_for}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    const MINUTE: i64 = 60_000;

    fn refused(sql: &str, dialect: Dialect) -> Option<String> {
        match refusal(sql, dialect)? {
            Refusal::Control(keywords) => Some(keywords),
            Refusal::ImplicitCommit(keywords) => panic!("{keywords} commits implicitly"),
        }
    }

    #[test]
    fn starts_in_manual_on_a_connection_whose_tag_asks_first() {
        assert_eq!(default_mode(Some("PROD")), TransactionMode::Manual);
        assert_eq!(default_mode(None), TransactionMode::Auto);
    }

    #[test]
    fn finds_statements_that_start_or_end_a_transaction() {
        let pg = Dialect::Postgres;
        assert_eq!(refused("commit", pg).as_deref(), Some("COMMIT"));
        assert_eq!(refused("UPDATE t SET a = 1;\n  RollBack;", pg).as_deref(), Some("ROLLBACK"));
        assert_eq!(refused("BEGIN; UPDATE t SET a = 1", pg).as_deref(), Some("BEGIN"));
        assert_eq!(refused("start transaction read only", pg).as_deref(), Some("START TRANSACTION"));
        assert_eq!(refused("END", pg).as_deref(), Some("END"));
        assert_eq!(refused("abort", pg).as_deref(), Some("ABORT"));
        assert_eq!(refused("-- done\n/* really */ COMMIT", pg).as_deref(), Some("COMMIT"), "after comments");
        assert_eq!(refused("/* a\nlong one */\ncommit", pg).as_deref(), Some("COMMIT"));
    }

    #[test]
    fn leaves_strings_comments_savepoints_and_other_statements_alone() {
        let pg = Dialect::Postgres;
        assert_eq!(refused("SELECT 'COMMIT'; UPDATE t SET note = 'rollback'", pg), None);
        assert_eq!(refused("-- COMMIT\nSELECT 1", pg), None);
        assert_eq!(refused("SELECT \"commit\" FROM t", pg), None);
        assert_eq!(refused("SAVEPOINT a; ROLLBACK TO a; RELEASE a", pg), None);
        assert_eq!(refused("SELECT begin_at, end_at FROM shifts", pg), None);
        assert_eq!(refused("START REPLICA", Dialect::Mysql), None);
        assert_eq!(refused("", pg), None);
    }

    #[test]
    fn reads_each_dialect_s_own_transaction_statements() {
        let tsql = Dialect::Sqlserver;
        assert_eq!(refused("BEGIN TRAN", tsql).as_deref(), Some("BEGIN TRAN"));
        assert_eq!(refused("begin distributed transaction", tsql).as_deref(), Some("BEGIN DISTRIBUTED"));
        assert_eq!(refused("COMMIT TRANSACTION", tsql).as_deref(), Some("COMMIT"));
        assert_eq!(refused("BEGIN TRY SELECT 1 END TRY", tsql), None, "a block");
        assert_eq!(refused("END", tsql), None);
        assert_eq!(refused("BEGIN WORK", Dialect::Mysql).as_deref(), Some("BEGIN"));
        assert_eq!(refused("# done\nCOMMIT", Dialect::Mysql).as_deref(), Some("COMMIT"));
        assert_eq!(refused("END", Dialect::Mysql), None);
        assert_eq!(refused("END TRANSACTION", Dialect::Sqlite).as_deref(), Some("END"));
        assert_eq!(refused("ABORT", Dialect::Sqlite), None);
        assert_eq!(refused("COMMIT", Dialect::Cosmos), None);
    }

    fn implicit(sql: &str) -> Option<Refusal> {
        refusal(sql, Dialect::Mysql)
    }

    #[test]
    fn refuses_what_commits_implicitly_on_mysql_and_mariadb() {
        let commits = |keywords: &str| Some(Refusal::ImplicitCommit(keywords.to_owned()));
        assert_eq!(implicit("CREATE TABLE a (id INT)"), commits("CREATE"));
        assert_eq!(implicit("UPDATE t SET a = 1;\n alter table t add b int"), commits("ALTER"));
        assert_eq!(implicit("-- old\nDROP INDEX i ON t"), commits("DROP"));
        for (sql, keywords) in [
            ("RENAME TABLE a TO b", "RENAME"),
            ("truncate t", "TRUNCATE"),
            ("GRANT SELECT ON t TO u", "GRANT"),
            ("REVOKE ALL ON t FROM u", "REVOKE"),
            ("LOCK TABLES t WRITE", "LOCK"),
            ("UNLOCK TABLES", "UNLOCK"),
            ("ANALYZE TABLE t", "ANALYZE"),
            ("OPTIMIZE TABLE t", "OPTIMIZE"),
            ("REPAIR TABLE t", "REPAIR"),
            ("CHECK TABLE t", "CHECK"),
            ("FLUSH PRIVILEGES", "FLUSH"),
            ("INSTALL PLUGIN p SONAME 'p.so'", "INSTALL"),
            ("UNINSTALL PLUGIN p", "UNINSTALL"),
            ("SET autocommit = 1", "SET AUTOCOMMIT"),
            ("set @@autocommit=1", "SET AUTOCOMMIT"),
            ("SET SESSION autocommit = 1", "SET AUTOCOMMIT"),
        ] {
            assert_eq!(implicit(sql), commits(keywords), "{sql}");
        }
    }

    #[test]
    fn lets_temporary_tables_and_other_statements_run_in_manual_on_mysql() {
        assert_eq!(implicit("CREATE TEMPORARY TABLE tmp (id INT)"), None);
        assert_eq!(implicit("drop temporary table tmp"), None);
        assert_eq!(implicit("SET @x = 1; SET NAMES utf8mb4; INSERT INTO t VALUES (1)"), None);
        assert_eq!(implicit("SELECT 'CREATE TABLE x'; UPDATE t SET note = 'drop'"), None);
        assert_eq!(implicit("/* CREATE */ SELECT 1"), None);
        // Transaction control is still refused as such.
        assert_eq!(implicit("COMMIT"), Some(Refusal::Control("COMMIT".into())));
    }

    #[test]
    fn other_dialects_keep_ddl_in_the_transaction() {
        for dialect in [Dialect::Postgres, Dialect::Sqlite, Dialect::Sqlserver] {
            assert_eq!(
                refusal("CREATE TABLE a (id INT); DROP TABLE a; TRUNCATE TABLE t", dialect),
                None,
                "{dialect:?}"
            );
        }
    }

    #[test]
    fn writes_how_long_as_a_clock() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(222_000), "3:42");
        assert_eq!(clock(612_999), "10:12");
        assert_eq!(clock(3_725_000), "1:02:05");
        assert_eq!(clock(-5), "0:00", "a clock behind the server's never goes negative");
    }

    #[test]
    fn warns_of_an_idle_transaction_every_ten_minutes() {
        assert_eq!(idle_warnings(10 * MINUTE - 1), 0);
        assert_eq!(idle_warnings(10 * MINUTE), 1);
        assert_eq!(idle_warnings(19 * MINUTE), 1);
        assert_eq!(idle_warnings(20 * MINUTE), 2);
        assert_eq!(idle_warnings(30 * MINUTE), 3);
        assert_eq!(idle_minutes(3), 30);
    }

    fn transaction(started: i64, last: i64, aborted: bool) -> OpenTransaction {
        OpenTransaction { started_at_ms: started, last_activity_ms: last, changes: 2, rows: 1310, aborted }
    }

    #[test]
    fn the_capsule_says_what_is_pending_and_for_how_long() {
        let m = &Language::En.messages().transactions;
        let open = capsule(m, &transaction(0, 200_000, false), 222_000);
        assert_eq!(
            open,
            Capsule {
                tone: CapsuleTone::Open,
                title: "Transaction open".into(),
                detail: "2 changes · 1,310 rows · 3:42".into()
            }
        );
        let one = OpenTransaction { changes: 1, rows: 1, ..transaction(0, 0, false) };
        assert_eq!(capsule(m, &one, 1000).detail, "1 change · 1 row · 0:01");
        let idle = capsule(m, &transaction(0, 60_000, false), 21 * MINUTE);
        assert_eq!((idle.tone, idle.title.as_str()), (CapsuleTone::Idle, "Open 20 min · holds locks"));
        let failed = capsule(m, &transaction(0, 0, true), 30 * MINUTE);
        assert_eq!(
            (failed.tone, failed.title.as_str(), failed.detail.as_str()),
            (CapsuleTone::Failed, "Transaction failed", "only Rollback works")
        );
        let es = capsule(&Language::Es.messages().transactions, &transaction(0, 0, false), 1000);
        assert_eq!(es.title, "Transacción abierta");
    }
}
