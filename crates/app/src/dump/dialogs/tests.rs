use dbforge_core::connections::model::ConnectionInput;
use dbforge_core::dump::{DumpOptions, MongodbOptions, MysqlOptions, PostgresOptions, SqliteOptions};
use gpui::{TestAppContext, VisualTestContext};

use super::*;
use crate::test_support::{draw, setup, wait_until};

/// A saved connection of `engine` tagged `environment`, its database `database`.
fn saved(engine: &str, environment: &str, database: &str, cx: &mut TestAppContext) -> Connection {
    cx.update(|cx| {
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "Shop", "engine": engine, "environment": environment, "host": "127.0.0.1",
            "port": 1, "database": database, "username": "nobody",
        }))
        .expect("an input");
        Backend::global(cx).core().connections().save(input.into_parts().expect("valid")).expect("saved")
    })
}

fn postgres(environment: &str, cx: &mut TestAppContext) -> Connection {
    saved("postgres", environment, "shop", cx)
}

fn file_name(dialog: &Entity<ExportDatabaseDialog>, cx: &mut VisualTestContext) -> String {
    dialog.read_with(cx, |dialog, cx| dialog.file_name.read(cx).text().to_owned())
}

#[gpui::test]
fn a_restore_on_a_connection_whose_tag_asks_first_only_warns_on_the_first_press(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = postgres("production", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ImportDatabaseDialog::new(connection, "shop".into(), window, cx));
    dialog.update_in(cx, |dialog, window, cx| {
        dialog.start(window, cx);
        assert!(!dialog.confirming && !dialog.job.running(), "no file chosen: nothing happens");
        dialog.path = Some("/nowhere/shop.sql".into());
        dialog.start(window, cx);
        assert!(dialog.confirming, "the first press only warns");
        assert!(!dialog.job.running(), "and runs nothing");
        dialog.target.update(cx, |input, cx| input.insert("_copy", cx));
    });
    cx.run_until_parked();
    dialog.read_with(cx, |dialog, _| assert!(!dialog.confirming, "another database asks again"));
    draw(cx);
}

#[gpui::test]
fn an_export_names_its_file_after_the_database_and_the_minute(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = postgres("development", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ExportDatabaseDialog::new(connection, "shop".into(), window, cx));
    let name = file_name(&dialog, cx);
    assert!(name.starts_with("shop-") && name.ends_with(".sql"), "{name}");
    let request = dialog.read_with(cx, |dialog, _| dialog.request("job".into(), "/out/shop.sql".into()));
    assert!(matches!(request.options, DumpOptions::Postgres(PostgresOptions { content: Content::All, .. })));
    draw(cx);
}

#[gpui::test]
fn a_mysql_export_asks_mysqldump_for_a_consistent_snapshot_with_routines(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = saved("mariadb", "development", "shop", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ExportDatabaseDialog::new(connection, "shop".into(), window, cx));
    dialog.update_in(cx, |dialog, _, cx| {
        assert_eq!(dialog.tool.as_ref().map(|tool| tool.read(cx).name()), Some("mysqldump"));
        dialog.change(cx, |choices| {
            if let ExportChoices::Mysql(options) = choices {
                options.content = Content::Structure;
                options.add_drop_table = true;
            }
        });
    });
    assert!(file_name(&dialog, cx).ends_with(".sql"));
    let request = dialog.read_with(cx, |dialog, _| dialog.request("job".into(), "/out/shop.sql".into()));
    assert!(matches!(
        request.options,
        DumpOptions::Mysql(MysqlOptions {
            content: Content::Structure,
            add_drop_table: true,
            single_transaction: true,
            routines: true
        })
    ));
    draw(cx);
}

#[gpui::test]
fn a_mongodb_export_writes_a_gzipped_archive_of_the_chosen_collections(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = saved("mongodb", "development", "shop", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ExportDatabaseDialog::new(connection, "shop".into(), window, cx));
    let name = file_name(&dialog, cx);
    assert!(name.starts_with("shop-") && name.ends_with(".archive.gz"), "{name}");
    dialog.update_in(cx, |dialog, _, cx| {
        assert_eq!(dialog.tool.as_ref().map(|tool| tool.read(cx).name()), Some("mongodump"));
        dialog.collections = Listing::Listed(vec!["orders".into(), "users".into()]);
        dialog.change(cx, |choices| {
            if let ExportChoices::Mongodb(collections) = choices {
                collections.whole = false;
            }
        });
        assert!(!dialog.ready(cx), "no collection chosen yet");
        dialog.change(cx, |choices| {
            if let ExportChoices::Mongodb(collections) = choices {
                collections.toggle("users", true);
            }
        });
        assert!(dialog.ready(cx));
    });
    draw(cx);
    let request = dialog.read_with(cx, |dialog, _| dialog.request("job".into(), "/out/shop.archive.gz".into()));
    assert!(
        matches!(request.options, DumpOptions::Mongodb(MongodbOptions { collections }) if collections == ["users"])
    );
}

#[gpui::test]
fn a_sqlite_export_is_named_after_its_file_and_switches_extension_with_its_format(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = saved("sqlite", "development", "/data/shop.db", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ExportDatabaseDialog::new(connection, "Shop".into(), window, cx));
    let name = file_name(&dialog, cx);
    assert!(name.starts_with("shop-") && name.ends_with(".sql"), "{name}");
    dialog.update_in(cx, |dialog, _, cx| {
        assert!(dialog.tool.is_none(), "SQLite needs no tool");
        dialog.change(cx, |choices| *choices = ExportChoices::Sqlite(SqliteFormat::File));
    });
    let name = file_name(&dialog, cx);
    assert!(name.starts_with("shop-") && name.ends_with(".db"), "{name}");
    let request = dialog.read_with(cx, |dialog, _| dialog.request("job".into(), "/out/shop.db".into()));
    assert!(matches!(request.options, DumpOptions::Sqlite(SqliteOptions { format: SqliteFormat::File })));
    draw(cx);
}

#[gpui::test]
fn a_sqlite_restore_goes_into_the_connection_s_file_or_the_new_one_chosen(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = saved("sqlite", "development", "/data/shop.db", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ImportDatabaseDialog::new(connection, "Shop".into(), window, cx));
    dialog.update_in(cx, |dialog, _, cx| {
        assert!(dialog.tool.is_none(), "SQLite needs no tool");
        dialog.path = Some("/in/shop.sql".into());
        let request = dialog.request("job".into(), cx).expect("the connection's file");
        assert_eq!(request.database, "/data/shop.db");
        dialog.into_new_file = true;
        assert!(dialog.request("job".into(), cx).is_none() && !dialog.ready(cx), "a new file is picked first");
        dialog.new_file = Some("/data/copy.db".into());
        assert_eq!(dialog.request("job".into(), cx).expect("the new file").database, "/data/copy.db");
        assert_eq!(dialog.shown("/data/copy.db"), "copy.db");
    });
    draw(cx);
}

#[gpui::test]
fn a_mongodb_restore_drops_existing_collections_only_when_asked(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let connection = saved("mongodb", "development", "shop", cx);
    let (dialog, cx) =
        cx.add_window_view(|window, cx| ImportDatabaseDialog::new(connection, "shop".into(), window, cx));
    dialog.update_in(cx, |dialog, _, cx| {
        assert_eq!(dialog.tool.as_ref().map(|tool| tool.read(cx).name()), Some("mongorestore"));
        dialog.path = Some("/in/shop.archive.gz".into());
        assert!(!dialog.request("job".into(), cx).expect("a request").options.drop_existing);
        dialog.drop_existing = true;
        let request = dialog.request("job".into(), cx).expect("a request");
        assert!(request.options.drop_existing);
        assert_eq!(request.database, "shop");
    });
    draw(cx);
}

/// A SQLite script into an empty file through the import dialog, that file
/// out again through the export dialog, and the export into a new file.
#[gpui::test]
fn a_sqlite_database_goes_out_to_a_script_and_back_into_a_new_file(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let file = dir.path().join("shop.db");
    // An empty file is an empty SQLite database.
    std::fs::File::create(&file).expect("the database file");
    let script = dir.path().join("seed.sql");
    std::fs::write(&script, "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT);\nINSERT INTO people (name) VALUES ('Ada'), ('Grace');\n")
        .expect("the script");
    let connection = saved("sqlite", "development", &file.display().to_string(), cx);

    let (import, cx) = cx.add_window_view({
        let connection = connection.clone();
        move |window, cx| ImportDatabaseDialog::new(connection, "Shop".into(), window, cx)
    });
    let done = |outcome: &Option<Outcome>| outcome.as_ref().is_some_and(|outcome| outcome.tone != Tone::Progress);
    import.update_in(cx, |dialog, window, cx| {
        dialog.path = Some(script.display().to_string());
        dialog.start(window, cx);
    });
    wait_until(cx, "the seed", |cx| import.read_with(cx, |dialog, _| done(&dialog.outcome)));
    import.read_with(cx, |dialog, _| {
        assert_eq!(dialog.outcome.as_ref().map(|o| o.tone), Some(Tone::Done), "{:?}", dialog.outcome)
    });

    let exports = dir.path().join("exports");
    let export = cx.update(|window, cx| {
        let export = cx.new(|cx| ExportDatabaseDialog::new(connection.clone(), "Shop".into(), window, cx));
        export.update(cx, |dialog, _| dialog.folder = Some(exports.display().to_string()));
        export
    });
    export.update_in(cx, |dialog, window, cx| dialog.start(window, cx));
    wait_until(cx, "the export", |cx| export.read_with(cx, |dialog, _| done(&dialog.outcome)));
    let written = export.read_with(cx, |dialog, _| {
        let outcome = dialog.outcome.clone().expect("an outcome");
        assert_eq!(outcome.tone, Tone::Done, "{outcome:?}");
        outcome.detail.expect("the path")
    });
    assert!(written.ends_with(".sql"), "{written}");
    let dumped = std::fs::read_to_string(&written).expect("the export");
    assert!(dumped.contains("people") && dumped.contains("Grace"), "{dumped}");

    let copy = dir.path().join("copy.db");
    import.update_in(cx, |dialog, window, cx| {
        dialog.path = Some(written.clone());
        dialog.into_new_file = true;
        dialog.new_file = Some(copy.display().to_string());
        dialog.outcome = None;
        dialog.start(window, cx);
    });
    wait_until(cx, "the restore", |cx| import.read_with(cx, |dialog, _| done(&dialog.outcome)));
    import.read_with(cx, |dialog, _| {
        let outcome = dialog.outcome.clone().expect("an outcome");
        assert_eq!(outcome.tone, Tone::Done, "{outcome:?}");
        assert!(outcome.title.starts_with("Created copy.db"), "{}", outcome.title);
    });
    assert!(std::fs::metadata(&copy).expect("the new file").len() > 0);
    draw(cx);
}
