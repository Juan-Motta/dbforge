//! MongoDB, with the Database Tools: mongodump writes one database to a
//! gzipped archive, mongorestore restores it into a database of any name.
//!
//! The tools read the connection string and password from a config file only
//! their owner can read (`--config`), deleted after the run, never from their
//! command line. The string is built as `drivers::mongodb` connects: a saved
//! `mongodb://` or `mongodb+srv://` URL as it is, SRV seed lists by name,
//! other servers through the SSH tunnel's route, directly, users signing in
//! against `admin`.
//!
//! A restore renames the archive's database to the target with
//! `--nsFrom`/`--nsTo`. The source database is read from the archive itself
//! (its first collection's header), so a renamed file still restores. A
//! database created for a failed restore is dropped again.

use std::io::Read;
use std::path::Path;

use mongodb::bson::Document;
use serde::Deserialize;
use tokio::process::Command;

use super::secret_file::SecretFile;
use super::tools::{self, Tool};
use super::{RestoreOptions, Restored, Run, blocking, stop_if};
use crate::connections::model::Connection;
use crate::drivers::mongodb as driver;
use crate::error::{AppError, Result};
use crate::tunnel::Route;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MongodbOptions {
    /// The collections to export; all of them when empty.
    pub collections: Vec<String>,
}

/// Writes `database` (or the chosen collections) to `file` as a gzipped archive with mongodump.
pub(super) async fn dump(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    file: &Path,
    options: &MongodbOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<()> {
    let tool = run.tool()?;
    check_name(database)?;
    let only = if options.collections.is_empty() {
        Vec::new()
    } else {
        let existing: Vec<String> = driver::list_objects(connection, password, &[database.to_string()])
            .await?
            .into_iter()
            .map(|object| object.name)
            .collect();
        collection_args(&options.collections, &existing, database)?
    };
    stop_if(run.cancel)?;
    let config = config_file(connection, password).await?;
    let mut command = command(tool, &config);
    command.arg(format!("--db={database}")).arg(archive_arg(file)).arg("--gzip").args(only);
    tools::run(command, run.cancel, on_line).await
}

/// Restores the archive at `path` into `database`, which is created when it
/// doesn't exist, and dropped again when the restore fails.
pub(super) async fn restore(
    connection: &Connection,
    password: Option<&str>,
    database: &str,
    path: &Path,
    options: &RestoreOptions,
    run: Run<'_>,
    on_line: impl FnMut(&str),
) -> Result<Restored> {
    let tool = run.tool()?;
    check_name(database)?;
    let file = path.to_path_buf();
    let source = blocking(move || archive_database(std::fs::File::open(&file)?))
        .await?
        .ok_or_else(|| AppError::Invalid(format!("{} holds no collections to restore.", path.display())))?;
    stop_if(run.cancel)?;
    let created = !driver::database_exists(connection, password, database).await?;
    stop_if(run.cancel)?;
    let outcome = async {
        let config = config_file(connection, password).await?;
        let mut command = command(tool, &config);
        command.arg(archive_arg(path)).arg("--gzip").args(restore_args(&source, database, options.drop_existing));
        tools::run(command, run.cancel, on_line).await
    }
    .await;
    let Err(error) = outcome else { return Ok(Restored::created(created)) };
    // mongorestore creates the database with its first collection, if it got that far.
    if created {
        let dropped = async {
            if driver::database_exists(connection, password, database).await? {
                driver::drop_database(connection, password, database).await?;
            }
            Ok::<_, AppError>(())
        };
        if let Err(problem) = dropped.await {
            return Err(AppError::Invalid(format!(
                "{error}\nThe database {database} created for the import couldn't be removed: {problem}"
            )));
        }
    }
    Err(error)
}

/// `tool`, reading the connection from `config`.
fn command(tool: &Tool, config: &SecretFile) -> Command {
    let mut command = Command::new(&tool.path);
    let mut option = std::ffi::OsString::from("--config=");
    option.push(config.path());
    command.arg(option);
    command
}

fn archive_arg(file: &Path) -> std::ffi::OsString {
    let mut arg = std::ffi::OsString::from("--archive=");
    arg.push(file);
    arg
}

/// The config file the tools sign in with: the connection string and, when
/// the string doesn't hold one, the password. Its values are written as JSON
/// strings, which YAML reads as they are.
async fn config_file(connection: &Connection, password: Option<&str>) -> Result<SecretFile> {
    let route =
        if is_uri(&connection.host) || connection.srv { None } else { Some(crate::tunnel::route(connection).await?) };
    let uri = connection_uri(connection, route.as_ref());
    SecretFile::new(&config_text(&uri, password), ".yaml")
}

fn config_text(uri: &str, password: Option<&str>) -> String {
    let quote = |value: &str| serde_json::to_string(value).expect("a string");
    let mut text = format!("uri: {}\n", quote(uri));
    if let Some(password) = password.filter(|_| !uri_has_password(uri)) {
        text.push_str(&format!("password: {}\n", quote(password)));
    }
    text
}

fn is_uri(host: &str) -> bool {
    host.starts_with("mongodb://") || host.starts_with("mongodb+srv://")
}

/// The connection string for the tools: the saved one without its database
/// (the tools refuse one that differs from `--db`), else one built from the
/// connection's fields with `route` (the server or its tunnel).
fn connection_uri(connection: &Connection, route: Option<&Route>) -> String {
    if is_uri(&connection.host) {
        return without_database(&connection.host);
    }
    let user =
        if connection.username.is_empty() { String::new() } else { format!("{}@", encode(&connection.username)) };
    let mut options: Vec<String> = Vec::new();
    let mut uri = match route {
        Some(route) if !connection.srv => {
            // Directly to this server, even when it belongs to a replica set whose
            // members advertise names only reachable from inside their network.
            options.push("directConnection=true".into());
            let host = if route.host.contains(':') { format!("[{}]", route.host) } else { route.host.clone() };
            format!("mongodb://{user}{host}:{}/", route.port)
        }
        _ => format!("mongodb+srv://{user}{}/", connection.host),
    };
    if !connection.username.is_empty() {
        options.push("authSource=admin".into());
    }
    let ssl = &connection.ssl;
    if ssl.mode.encrypts() {
        options.push("tls=true".into());
        if !ssl.mode.verifies() {
            options.push("tlsInsecure=true".into());
        }
        if !ssl.ca_file.is_empty() {
            options.push(format!("tlsCAFile={}", encode(&ssl.ca_file)));
        }
        if !ssl.cert_file.is_empty() {
            // One file with the certificate and its key, as for the driver.
            options.push(format!("tlsCertificateKeyFile={}", encode(&ssl.cert_file)));
        }
    }
    if !options.is_empty() {
        uri.push('?');
        uri.push_str(&options.join("&"));
    }
    uri
}

/// `uri` without the database in its path. It was also the database users
/// sign in against when the string names none, so that stays as `authSource`.
fn without_database(uri: &str) -> String {
    let (scheme, rest) = uri.split_once("://").expect("a URI");
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (path, query) = match tail.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (tail, None),
    };
    let database = path.trim_start_matches('/');
    let mut options: Vec<String> =
        query.into_iter().flat_map(|q| q.split('&')).filter(|o| !o.is_empty()).map(String::from).collect();
    let signs_in = authority.contains('@');
    if !database.is_empty() && signs_in && !options.iter().any(|o| o.to_ascii_lowercase().starts_with("authsource=")) {
        options.push(format!("authSource={database}"));
    }
    let mut uri = format!("{scheme}://{authority}/");
    if !options.is_empty() {
        uri.push('?');
        uri.push_str(&options.join("&"));
    }
    uri
}

/// Whether `uri` holds a password (`user:password@`).
fn uri_has_password(uri: &str) -> bool {
    let rest = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let authority = &rest[..rest.find(['/', '?']).unwrap_or(rest.len())];
    authority.rsplit_once('@').is_some_and(|(user, _)| user.contains(':'))
}

/// `text` percent-encoded for a connection string: all but unreserved characters and `/`.
fn encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// MongoDB's rules for a database name, checked before a tool runs.
fn check_name(database: &str) -> Result<()> {
    const NOT_ALLOWED: &[char] = &['/', '\\', '.', ' ', '"', '$', '*', '<', '>', ':', '|', '?', '\0'];
    if database.is_empty() || database.len() >= 64 || database.contains(NOT_ALLOWED) {
        return Err(AppError::Invalid(format!(
            "“{database}” can't be a MongoDB database name: it needs 1 to 63 characters, none of / \\ . \" $ * < > : | ? or spaces."
        )));
    }
    Ok(())
}

/// mongodump's options to export only `chosen` of the `existing` collections:
/// one is named, several are what's left after excluding the others.
fn collection_args(chosen: &[String], existing: &[String], database: &str) -> Result<Vec<String>> {
    if let Some(missing) = chosen.iter().find(|name| !existing.contains(name)) {
        return Err(AppError::Invalid(format!("There's no collection {missing} in {database}.")));
    }
    if let [only] = chosen {
        return Ok(vec![format!("--collection={only}")]);
    }
    Ok(existing
        .iter()
        .filter(|name| !chosen.contains(name))
        .map(|name| format!("--excludeCollection={name}"))
        .collect())
}

/// mongorestore's options to restore the archive's `source` database into `target`.
fn restore_args(source: &str, target: &str, drop: bool) -> Vec<String> {
    let source = escape_namespace(source);
    let mut args = vec![
        format!("--nsInclude={source}.*"),
        format!("--nsFrom={source}.$collection$"),
        format!("--nsTo={target}.$collection$"),
        "--stopOnError".to_string(),
    ];
    if drop {
        args.push("--drop".into());
    }
    args
}

/// `name` as a namespace pattern matches it literally: `*`, `$` and `\` escaped.
fn escape_namespace(name: &str) -> String {
    let mut escaped = String::new();
    for c in name.chars() {
        if matches!(c, '*' | '$' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// The magic number a mongodump archive starts with.
const ARCHIVE_MAGIC: u32 = 0x8199_e26d;

/// The database of the first collection in a gzipped mongodump archive, or
/// `None` when it holds none. An archive starts with the magic number, a
/// header document, then one document per collection (`db`, `collection`, …).
fn archive_database(file: impl Read) -> std::io::Result<Option<String>> {
    let invalid = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string());
    let not_archive = "This isn't a MongoDB archive (mongodump --archive --gzip).";
    let mut archive = flate2::read::GzDecoder::new(file);
    let mut magic = [0u8; 4];
    archive.read_exact(&mut magic).map_err(|_| invalid(not_archive))?;
    if u32::from_le_bytes(magic) != ARCHIVE_MAGIC {
        return Err(invalid(not_archive));
    }
    Document::from_reader(&mut archive).map_err(|_| invalid(not_archive))?;
    // The header list ends with a terminator (-1) where a document's length would be.
    let mut length = [0u8; 4];
    archive.read_exact(&mut length).map_err(|_| invalid(not_archive))?;
    if i32::from_le_bytes(length) == -1 {
        return Ok(None);
    }
    let size = i32::from_le_bytes(length);
    if !(5..=16 * 1024 * 1024).contains(&size) {
        return Err(invalid(not_archive));
    }
    let mut bytes = length.to_vec();
    bytes.resize(size as usize, 0);
    archive.read_exact(&mut bytes[4..]).map_err(|_| invalid(not_archive))?;
    let collection = Document::from_reader(bytes.as_slice()).map_err(|_| invalid(not_archive))?;
    Ok(collection.get_str("db").ok().map(String::from))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use mongodb::bson::doc;

    use super::*;
    use crate::connections::transport::{Ssl, SslMode};
    use crate::drivers::mongodb::tests::{PASSWORD, local};

    fn route() -> Route {
        Route { host: "127.0.0.1".into(), port: 41234 }
    }

    #[test]
    fn the_uri_goes_to_the_route_directly_and_signs_in_against_admin() {
        let connection = Connection { host: "mongo.internal".into(), username: "app user".into(), ..local() };
        assert_eq!(
            connection_uri(&connection, Some(&route())),
            "mongodb://app%20user@127.0.0.1:41234/?directConnection=true&authSource=admin"
        );
        let anonymous = Connection { username: String::new(), ..connection.clone() };
        assert_eq!(connection_uri(&anonymous, Some(&route())), "mongodb://127.0.0.1:41234/?directConnection=true");
        let srv = Connection { host: "cluster0.example.net".into(), srv: true, ..connection };
        assert_eq!(connection_uri(&srv, None), "mongodb+srv://app%20user@cluster0.example.net/?authSource=admin");
    }

    #[test]
    fn tls_settings_go_in_the_uri() {
        let ssl = |mode| Ssl {
            mode,
            ca_file: "/certs/my ca.pem".into(),
            cert_file: "/certs/client.pem".into(),
            ..Ssl::default()
        };
        let tls = |mode| connection_uri(&Connection { ssl: ssl(mode), ..local() }, Some(&route()));
        assert_eq!(
            tls(SslMode::VerifyFull),
            "mongodb://root@127.0.0.1:41234/?directConnection=true&authSource=admin&tls=true\
             &tlsCAFile=/certs/my%20ca.pem&tlsCertificateKeyFile=/certs/client.pem"
        );
        assert!(tls(SslMode::Require).contains("&tls=true&tlsInsecure=true&"), "{}", tls(SslMode::Require));
        assert!(!tls(SslMode::Disable).contains("tls"));
    }

    #[test]
    fn a_saved_uri_is_used_without_its_database() {
        let saved = |host: &str| connection_uri(&Connection { host: host.into(), ..local() }, None);
        assert_eq!(
            saved("mongodb://u:p@h1,h2/shop?replicaSet=rs"),
            "mongodb://u:p@h1,h2/?replicaSet=rs&authSource=shop"
        );
        assert_eq!(
            saved("mongodb+srv://u@c.example.net/shop?authSource=admin"),
            "mongodb+srv://u@c.example.net/?authSource=admin"
        );
        assert_eq!(saved("mongodb://h:27017"), "mongodb://h:27017/");
        assert_eq!(saved("mongodb://h/shop"), "mongodb://h/", "no one signs in");
    }

    #[test]
    fn the_password_goes_in_the_config_file_unless_the_uri_has_one() {
        assert_eq!(
            config_text("mongodb://root@h/", Some("it's \"x\"\n")),
            "uri: \"mongodb://root@h/\"\npassword: \"it's \\\"x\\\"\\n\"\n"
        );
        assert_eq!(config_text("mongodb://u:p@h/", Some("other")), "uri: \"mongodb://u:p@h/\"\n");
        assert!(uri_has_password("mongodb://u:p%40ss@h:1/?x=y:z"));
        assert!(!uri_has_password("mongodb://u@h:1/"));
        assert!(!uri_has_password("mongodb://h:1/?a=b@c:d"));
    }

    #[tokio::test]
    async fn the_command_holds_no_password() {
        let connection = Connection { ssh: Default::default(), ..local() };
        let config = config_file(&connection, Some("s3cret")).await.unwrap();
        let text = std::fs::read_to_string(config.path()).unwrap();
        assert!(text.contains("s3cret") && text.contains("mongodb://root@127.0.0.1:57017/"), "{text}");
        let tool = Tool { path: PathBuf::from("/bin/mongodump"), version: (100, 10), custom: false, mariadb: false };
        let command = command(&tool, &config);
        let args: Vec<String> = command.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, [format!("--config={}", config.path().display())]);
    }

    #[test]
    fn restores_rename_the_archives_database() {
        assert_eq!(
            restore_args("shop", "shop_copy", false),
            ["--nsInclude=shop.*", "--nsFrom=shop.$collection$", "--nsTo=shop_copy.$collection$", "--stopOnError"]
        );
        assert_eq!(
            restore_args("a*b", "c", true)[..2],
            ["--nsInclude=a\\*b.*".to_string(), "--nsFrom=a\\*b.$collection$".into()]
        );
        assert_eq!(restore_args("x", "y", true).last().unwrap(), "--drop");
    }

    #[test]
    fn database_names_follow_mongodbs_rules() {
        assert!(check_name("shop_copy-2").is_ok());
        for bad in ["", "a.b", "a b", "a$b", "a/b", "a\"b", &"x".repeat(64)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn chosen_collections_become_mongodump_options() {
        let existing = ["a".to_string(), "b".into(), "c".into()];
        assert_eq!(collection_args(&["b".into()], &existing, "db").unwrap(), ["--collection=b"]);
        assert_eq!(collection_args(&["a".into(), "c".into()], &existing, "db").unwrap(), ["--excludeCollection=b"]);
        let error = collection_args(&["z".into()], &existing, "db").unwrap_err();
        assert_eq!(error.to_string(), "There's no collection z in db.");
    }

    /// A gzipped archive as mongodump writes one: magic, header, collection headers, terminator.
    fn archive(databases: &[&str]) -> Vec<u8> {
        let mut raw = ARCHIVE_MAGIC.to_le_bytes().to_vec();
        let header = doc! { "concurrent_collections": 4, "version": "0.1", "server_version": "8.0.0", "tool_version": "100.10.0" };
        header.to_writer(&mut raw).unwrap();
        for db in databases {
            doc! { "db": *db, "collection": "items", "metadata": "{}", "size": 0_i64, "type": "collection" }
                .to_writer(&mut raw)
                .unwrap();
        }
        raw.extend((-1_i32).to_le_bytes());
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&raw).unwrap();
        gzip.finish().unwrap()
    }

    #[test]
    fn reads_the_database_an_archive_holds() {
        assert_eq!(archive_database(archive(&["shop"]).as_slice()).unwrap(), Some("shop".into()));
        assert_eq!(archive_database(archive(&[]).as_slice()).unwrap(), None);
        let error = archive_database(b"CREATE TABLE x;".as_slice()).unwrap_err();
        assert_eq!(error.to_string(), "This isn't a MongoDB archive (mongodump --archive --gzip).");
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(b"not an archive").unwrap();
        assert!(archive_database(gzip.finish().unwrap().as_slice()).is_err());
    }

    // Live: MongoDB on 57017 (see `drivers::mongodb::tests`) and the Database
    // Tools installed. `cargo test -- --ignored`.

    use crate::drivers::mongodb::tests::database;
    use crate::dump::tools::Flavour;
    use crate::dump::{DumpOptions, export, import};
    use tokio_util::sync::CancellationToken;

    async fn mongodump() -> Tool {
        tools::find("mongodump", Flavour::Standard).await.unwrap()
    }

    async fn mongorestore() -> Tool {
        tools::find("mongorestore", Flavour::Standard).await.unwrap()
    }

    async fn export_to(password: Option<&str>, database: &str, collections: &[&str], file: &Path) -> Result<()> {
        let tool = mongodump().await;
        let run = Run { tool: Some(&tool), cancel: &CancellationToken::new() };
        let options =
            DumpOptions::Mongodb(MongodbOptions { collections: collections.iter().map(|c| c.to_string()).collect() });
        export(&local(), password, database, file, &options, run, |_| {}).await.map(|_| ())
    }

    async fn restore_into(database: &str, file: &Path, drop: bool, cancel: &CancellationToken) -> Result<bool> {
        let tool = mongorestore().await;
        let run = Run { tool: Some(&tool), cancel };
        let options = RestoreOptions { drop_existing: drop };
        Ok(import(&local(), PASSWORD, database, file, &options, run, |_| {}).await?.created)
    }

    async fn documents(database: &str, collection: &str) -> u64 {
        let client = mongodb::Client::with_uri_str("mongodb://root:dbforge@127.0.0.1:57017/?directConnection=true")
            .await
            .unwrap();
        client.database(database).collection::<Document>(collection).count_documents(doc! {}).await.unwrap()
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017 and the Database Tools"]
    async fn exports_a_database_and_restores_it_into_a_new_one() {
        database(
            "dbforge_dump_source",
            &[
                ("items", vec![doc! { "n": 1 }, doc! { "n": 2 }]),
                ("fs.chunks", vec![doc! { "c": 1 }]),
                ("other", vec![doc! {}]),
            ],
        )
        .await;
        let _ = driver::drop_database(&local(), PASSWORD, "dbforge_dump_copy").await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new folder").join("source.archive.gz");
        export_to(PASSWORD, "dbforge_dump_source", &[], &file).await.unwrap();
        // Renamed, the file still restores: the database is read from the archive.
        let renamed = dir.path().join("renamed.archive.gz");
        std::fs::rename(&file, &renamed).unwrap();

        assert!(restore_into("dbforge_dump_copy", &renamed, false, &CancellationToken::new()).await.unwrap());
        assert_eq!(documents("dbforge_dump_copy", "items").await, 2);
        assert_eq!(documents("dbforge_dump_copy", "fs.chunks").await, 1, "dotted names keep their whole name");
        assert_eq!(documents("dbforge_dump_source", "items").await, 2, "the source is untouched");
        // Again with the collections dropped first: no duplicate keys.
        assert!(!restore_into("dbforge_dump_copy", &renamed, true, &CancellationToken::new()).await.unwrap());
        assert_eq!(documents("dbforge_dump_copy", "items").await, 2);

        // Chosen collections only.
        let some = dir.path().join("some.archive.gz");
        export_to(PASSWORD, "dbforge_dump_source", &["items", "other"], &some).await.unwrap();
        driver::drop_database(&local(), PASSWORD, "dbforge_dump_copy").await.unwrap();
        restore_into("dbforge_dump_copy", &some, false, &CancellationToken::new()).await.unwrap();
        assert_eq!(documents("dbforge_dump_copy", "fs.chunks").await, 0);
        assert_eq!(documents("dbforge_dump_copy", "items").await, 2);

        for name in ["dbforge_dump_source", "dbforge_dump_copy"] {
            driver::drop_database(&local(), PASSWORD, name).await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017 and the Database Tools"]
    async fn failed_or_stopped_runs_leave_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("kept.archive.gz");
        std::fs::write(&file, "an older dump").unwrap();
        let error = export_to(Some("wrong"), "shop", &[], &file).await.unwrap_err();
        assert!(error.to_string().contains("Failed"), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "an older dump");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no partial file is left");

        let error = restore_into("dbforge_dump_broken", &file, false, &CancellationToken::new()).await.unwrap_err();
        assert!(error.to_string().contains("isn't a MongoDB archive"), "{error}");

        // Stopped before mongorestore started: nothing is created.
        database("dbforge_dump_stopped_source", &[("items", vec![doc! { "_id": 1 }])]).await;
        let archive = dir.path().join("source.archive.gz");
        export_to(PASSWORD, "dbforge_dump_stopped_source", &[], &archive).await.unwrap();
        let _ = driver::drop_database(&local(), PASSWORD, "dbforge_dump_twice").await;
        let stopped = CancellationToken::new();
        stopped.cancel();
        assert_eq!(
            restore_into("dbforge_dump_twice", &archive, false, &stopped).await.unwrap_err().to_string(),
            "Cancelled."
        );
        assert!(!driver::database_exists(&local(), PASSWORD, "dbforge_dump_twice").await.unwrap());
        driver::drop_database(&local(), PASSWORD, "dbforge_dump_stopped_source").await.unwrap();
    }
}
