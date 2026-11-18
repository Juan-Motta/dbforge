//! Connection URLs both ways: the URL a connection points at, without its
//! password (copied from the explorer or shown in the dialog), and the form
//! values a pasted `redis://`, `mongodb://` or `mongodb+srv://` URL, or a
//! Cosmos DB connection string, stands for.

use std::sync::LazyLock;

use dbforge_core::connections::model::{Connection, Engine};
use dbforge_core::connections::transport::SslMode;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use regex::Regex;

use super::engine::default_port;
use super::form::{FormChanges, account_url, ssl_with};

/// What `encodeURIComponent` leaves as it is: letters, digits and `-_.!~*'()`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn encode(part: &str) -> String {
    utf8_percent_encode(part, COMPONENT).to_string()
}

/// `part` with its `%XX` escapes decoded; as it is when they don't decode.
fn decode(part: &str) -> String {
    let bytes = part.as_bytes();
    let well_formed = bytes.iter().enumerate().all(|(index, &byte)| {
        byte != b'%' || bytes.get(index + 1..index + 3).is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    });
    if !well_formed {
        return part.to_owned();
    }
    percent_decode_str(part).decode_utf8().map(|text| text.into_owned()).unwrap_or_else(|_| part.to_owned())
}

/// A URL (or, for Cosmos DB, a connection string) that points at the
/// connection, without its password or key, in a form the connection dialog
/// reads back when pasted. SQLite gives its file.
pub fn connection_url(connection: &Connection) -> String {
    let Connection { engine, host, port, username, database, .. } = connection;
    let user = if username.is_empty() { String::new() } else { format!("{}@", encode(username)) };
    let path = if database.is_empty() { String::new() } else { format!("/{}", encode(database)) };
    let mode = connection.ssl.mode;
    match engine {
        Engine::Sqlite => database.clone(),
        Engine::Cosmosdb => format!("AccountEndpoint={host};"),
        Engine::Sqlserver => {
            let mut parts = vec![format!("sqlserver://{host}:{port}")];
            if !database.is_empty() {
                parts.push(format!("database={database}"));
            }
            if !username.is_empty() {
                parts.push(format!("user={username}"));
            }
            parts.join(";")
        }
        Engine::Redis => {
            if host.starts_with("redis://") || host.starts_with("rediss://") {
                return without_password(host);
            }
            let scheme = if mode.encrypts() { "rediss" } else { "redis" };
            let insecure = if mode == SslMode::Require { "#insecure" } else { "" };
            format!("{scheme}://{user}{host}:{port}{path}{insecure}")
        }
        Engine::Mongodb => {
            if host.starts_with("mongodb://") || host.starts_with("mongodb+srv://") {
                return without_password(host);
            }
            let mut options = url::form_urlencoded::Serializer::new(String::new());
            if mode.encrypts() && !connection.srv {
                options.append_pair("tls", "true");
            }
            if mode == SslMode::Require {
                options.append_pair("tlsAllowInvalidCertificates", "true");
            }
            let options = options.finish();
            let query = if options.is_empty() { String::new() } else { format!("?{options}") };
            if connection.srv {
                format!("mongodb+srv://{user}{host}/{}{query}", encode(database))
            } else {
                format!("mongodb://{user}{host}:{port}/{}{query}", encode(database))
            }
        }
        Engine::Postgres | Engine::Mysql | Engine::Mariadb => {
            let scheme = match engine {
                Engine::Postgres => "postgresql",
                Engine::Mysql => "mysql",
                _ => "mariadb",
            };
            format!("{scheme}://{user}{host}:{port}{path}{}", ssl_query(*engine, mode))
        }
    }
}

/// The SSL mode as a URL option, left out when it is `prefer`, the drivers'
/// default. Certificate files and the SSH tunnel are this machine's, never in the URL.
fn ssl_query(engine: Engine, mode: SslMode) -> String {
    if mode == SslMode::Prefer {
        return String::new();
    }
    if engine == Engine::Postgres {
        let name = match mode {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        };
        return format!("?sslmode={name}");
    }
    // MySQL's names for the modes, as its `ssl-mode` option takes them.
    let name = match mode {
        SslMode::Disable => "DISABLED",
        SslMode::Prefer => "PREFERRED",
        SslMode::Require => "REQUIRED",
        SslMode::VerifyCa => "VERIFY_CA",
        SslMode::VerifyFull => "VERIFY_IDENTITY",
    };
    format!("?ssl-mode={name}")
}

/// A URL saved whole as the host, with any password taken out.
fn without_password(url: &str) -> String {
    static CREDENTIALS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^([a-z+]+://)([^:@/]*):[^@/]*@").expect("a valid pattern"));
    CREDENTIALS
        .replace(url, |captures: &regex::Captures| {
            let (scheme, user) = (&captures[1], &captures[2]);
            if user.is_empty() { scheme.to_owned() } else { format!("{scheme}{user}@") }
        })
        .into_owned()
}

/// What a pasted URL fills in, and what of it the form can't hold, so it can
/// be shown rather than dropped silently.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUrl {
    pub changes: FormChanges,
    pub ignored: Vec<String>,
}

/// MongoDB URL options the form has fields for; `authSource=admin` is what it uses anyway.
const MONGO_OPTIONS: [&str; 4] = ["tls", "ssl", "tlsAllowInvalidCertificates", "tlsInsecure"];

/// The form values a `redis://`, `rediss://`, `mongodb://` or
/// `mongodb+srv://` URL, or a Cosmos DB connection string, stands for;
/// `None` for anything else. `extra_hosts` names the hosts after the first,
/// in the language in use.
pub fn parse_connection_url(text: &str, extra_hosts: fn(&str) -> String) -> Option<ParsedUrl> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(redis|rediss|mongodb|mongodb\+srv)://(?:([^:@/]*)(?::([^@/]*))?@)?([^/?#]*)(?:/([^?#]*))?(?:\?([^#]*))?(?:#(.*))?$",
        )
        .expect("a valid pattern")
    });
    if let Some(cosmos) = parse_cosmos_connection_string(text) {
        return Some(cosmos);
    }
    let captures = PATTERN.captures(text.trim())?;
    let part = |index: usize| captures.get(index).map_or("", |found| found.as_str());
    let (scheme, user, pass, hosts, path, query, fragment) =
        (part(1), part(2), part(3), part(4), part(5), part(6), part(7));
    let mut hosts = hosts.split(',');
    let first = hosts.next().unwrap_or_default();
    let others: Vec<&str> = hosts.collect();
    let (host, port) = match first.rfind(':') {
        Some(cut) if cut > 0 && !first.ends_with(']') => (&first[..cut], &first[cut + 1..]),
        _ => (first, ""),
    };
    let mut ignored = if others.is_empty() { Vec::new() } else { vec![extra_hosts(&others.join(","))] };
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let on = |names: &[&str]| {
        names.iter().any(|name| params.iter().find(|(key, _)| key == name).is_some_and(|(_, value)| value == "true"))
    };

    if scheme.starts_with("redis") {
        ignored.extend(params.iter().map(|(key, value)| format!("{key}={value}")));
        if !fragment.is_empty() && fragment != "insecure" {
            ignored.push(format!("#{fragment}"));
        }
        let mode = match (scheme, fragment) {
            ("redis", _) => SslMode::Disable,
            (_, "insecure") => SslMode::Require,
            _ => SslMode::VerifyFull,
        };
        return Some(ParsedUrl {
            changes: FormChanges {
                engine: Engine::Redis,
                host: decode(host),
                port: Some(if port.is_empty() { default_port(Engine::Redis).to_string() } else { port.to_owned() }),
                username: decode(user),
                password: decode(pass),
                database: Some(decode(path)),
                ssl: Some(ssl_with(mode)),
                srv: false,
            },
            ignored,
        });
    }

    for (key, value) in &params {
        if MONGO_OPTIONS.contains(&key.as_str()) || (key == "authSource" && value == "admin") {
            continue;
        }
        ignored.push(format!("{key}={value}"));
    }
    let srv = scheme == "mongodb+srv";
    let mode = if !(srv || on(&["tls", "ssl"])) {
        SslMode::Disable
    } else if on(&["tlsAllowInvalidCertificates", "tlsInsecure"]) {
        SslMode::Require
    } else {
        SslMode::VerifyFull
    };
    Some(ParsedUrl {
        changes: FormChanges {
            engine: Engine::Mongodb,
            host: decode(host),
            port: Some(if port.is_empty() { default_port(Engine::Mongodb).to_string() } else { port.to_owned() }),
            username: decode(user),
            password: decode(pass),
            database: Some(decode(path)),
            ssl: Some(ssl_with(mode)),
            srv,
        },
        ignored,
    })
}

/// `AccountEndpoint=https://<account>.documents.azure.com:443/;AccountKey=…;`,
/// as the Azure portal and the emulator give it. Its key is the account key.
fn parse_cosmos_connection_string(text: &str) -> Option<ParsedUrl> {
    let (mut endpoint, mut key) = (None, None);
    let mut ignored = Vec::new();
    for part in text.trim().split(';').map(str::trim) {
        // A key ends in Base64 padding, so only the first `=` splits a setting.
        let Some(cut) = part.find('=').filter(|&cut| cut > 0) else { continue };
        match part[..cut].to_lowercase().as_str() {
            "accountendpoint" => endpoint = Some(&part[cut + 1..]),
            "accountkey" => key = Some(&part[cut + 1..]),
            _ => ignored.push(part.to_owned()),
        }
    }
    let endpoint = account_url(endpoint.unwrap_or_default())?;
    Some(ParsedUrl {
        changes: FormChanges {
            engine: Engine::Cosmosdb,
            host: endpoint.to_string(),
            port: None,
            username: String::new(),
            password: key.unwrap_or_default().to_owned(),
            database: None,
            ssl: None,
            srv: false,
        },
        ignored,
    })
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::AgentAccess;
    use dbforge_core::connections::transport::{SshTunnel, Ssl};

    use super::*;

    fn hosts(hosts: &str) -> String {
        format!("hosts {hosts}")
    }

    fn ssl(mode: SslMode) -> Ssl {
        ssl_with(mode)
    }

    fn base() -> Connection {
        Connection {
            id: "x".into(),
            name: "x".into(),
            engine: Engine::Postgres,
            environment: None,
            host: "db.internal".into(),
            port: 5432,
            database: "shop".into(),
            username: "app".into(),
            ssl: ssl(SslMode::Prefer),
            ssh: SshTunnel::default(),
            srv: false,
            agent_access: AgentAccess::None,
        }
    }

    #[test]
    fn writes_each_engine_s_url_without_a_password() {
        assert_eq!(connection_url(&base()), "postgresql://app@db.internal:5432/shop");
        let mariadb = Connection { engine: Engine::Mariadb, port: 3306, ..base() };
        assert_eq!(connection_url(&mariadb), "mariadb://app@db.internal:3306/shop");
        let mysql = Connection { engine: Engine::Mysql, username: "a b".into(), database: String::new(), ..base() };
        assert_eq!(connection_url(&mysql), "mysql://a%20b@db.internal:5432");
        let sqlserver = Connection { engine: Engine::Sqlserver, port: 1433, ..base() };
        assert_eq!(connection_url(&sqlserver), "sqlserver://db.internal:1433;database=shop;user=app");
        let sqlite = Connection { engine: Engine::Sqlite, database: "/data/app.db".into(), ..base() };
        assert_eq!(connection_url(&sqlite), "/data/app.db");
        let cosmos =
            Connection { engine: Engine::Cosmosdb, host: "https://acct.documents.azure.com:443/".into(), ..base() };
        assert_eq!(connection_url(&cosmos), "AccountEndpoint=https://acct.documents.azure.com:443/;");
    }

    #[test]
    fn writes_redis_and_mongodb_urls_the_dialog_reads_back() {
        let redis = Connection {
            engine: Engine::Redis,
            port: 6380,
            database: "2".into(),
            ssl: ssl(SslMode::Require),
            ..base()
        };
        assert_eq!(connection_url(&redis), "rediss://app@db.internal:6380/2#insecure");
        let parsed = parse_connection_url(&connection_url(&redis), hosts).unwrap().changes;
        assert_eq!((parsed.host.as_str(), parsed.port.as_deref()), ("db.internal", Some("6380")));
        assert_eq!((parsed.username.as_str(), parsed.database.as_deref()), ("app", Some("2")));
        assert_eq!(parsed.ssl, Some(ssl(SslMode::Require)));

        let mongo = Connection { engine: Engine::Mongodb, port: 27017, ssl: ssl(SslMode::VerifyFull), ..base() };
        assert_eq!(connection_url(&mongo), "mongodb://app@db.internal:27017/shop?tls=true");
        let parsed = parse_connection_url(&connection_url(&mongo), hosts).unwrap().changes;
        assert_eq!((parsed.host.as_str(), parsed.ssl), ("db.internal", Some(ssl(SslMode::VerifyFull))));
        let insecure = Connection { ssl: ssl(SslMode::Require), ..mongo.clone() };
        assert_eq!(
            connection_url(&insecure),
            "mongodb://app@db.internal:27017/shop?tls=true&tlsAllowInvalidCertificates=true"
        );
        let plain = Connection { ssl: ssl(SslMode::Disable), ..mongo.clone() };
        assert_eq!(connection_url(&plain), "mongodb://app@db.internal:27017/shop");
        let srv = Connection { srv: true, host: "cluster.example.net".into(), ..mongo };
        assert_eq!(connection_url(&srv), "mongodb+srv://app@cluster.example.net/shop");
    }

    #[test]
    fn names_the_ssl_mode_unless_it_is_prefer() {
        let pg = |mode| connection_url(&Connection { ssl: ssl(mode), ..base() });
        assert_eq!(pg(SslMode::VerifyFull), "postgresql://app@db.internal:5432/shop?sslmode=verify-full");
        assert_eq!(pg(SslMode::Disable), "postgresql://app@db.internal:5432/shop?sslmode=disable");
        for (mode, name) in [
            (SslMode::Disable, "DISABLED"),
            (SslMode::Require, "REQUIRED"),
            (SslMode::VerifyCa, "VERIFY_CA"),
            (SslMode::VerifyFull, "VERIFY_IDENTITY"),
        ] {
            let mysql = Connection { engine: Engine::Mysql, port: 3306, ssl: ssl(mode), ..base() };
            assert_eq!(connection_url(&mysql), format!("mysql://app@db.internal:3306/shop?ssl-mode={name}"));
        }
    }

    #[test]
    fn never_includes_certificate_files_or_the_tunnel() {
        let connection = Connection {
            ssl: Ssl {
                mode: SslMode::VerifyCa,
                ca_file: "/ca.pem".into(),
                cert_file: "/c.pem".into(),
                key_file: "/c.key".into(),
            },
            ssh: SshTunnel { enabled: true, host: "bastion".into(), username: "ops".into(), ..SshTunnel::default() },
            ..base()
        };
        assert_eq!(connection_url(&connection), "postgresql://app@db.internal:5432/shop?sslmode=verify-ca");
    }

    #[test]
    fn takes_the_password_out_of_a_url_saved_as_the_host() {
        let mongo = Connection { engine: Engine::Mongodb, host: "mongodb://app:s3cret@h:27017/shop".into(), ..base() };
        assert_eq!(connection_url(&mongo), "mongodb://app@h:27017/shop");
        let redis = Connection { engine: Engine::Redis, host: "rediss://:s3cret@h:6380".into(), ..base() };
        assert_eq!(connection_url(&redis), "rediss://h:6380");
    }

    #[test]
    fn splits_a_redis_url_into_fields_tls_included() {
        let parsed = parse_connection_url("rediss://app:s%40cret@cache.example.com:6380/2#insecure", hosts).unwrap();
        assert_eq!(
            parsed,
            ParsedUrl {
                changes: FormChanges {
                    engine: Engine::Redis,
                    host: "cache.example.com".into(),
                    port: Some("6380".into()),
                    username: "app".into(),
                    password: "s@cret".into(),
                    database: Some("2".into()),
                    ssl: Some(ssl(SslMode::Require)),
                    srv: false,
                },
                ignored: Vec::new(),
            }
        );
        assert_eq!(
            parse_connection_url("rediss://cache:6380", hosts).unwrap().changes.ssl,
            Some(ssl(SslMode::VerifyFull))
        );
        let plain = parse_connection_url("redis://localhost", hosts).unwrap().changes;
        assert_eq!(plain.port.as_deref(), Some("6379"));
        assert_eq!(plain.ssl, Some(ssl(SslMode::Disable)));
        assert_eq!((plain.database.as_deref(), plain.username.as_str()), (Some(""), ""));
    }

    #[test]
    fn reads_mongodb_seed_lists_and_tls_options_and_lists_what_it_leaves_out() {
        let atlas = parse_connection_url(
            "mongodb+srv://ana:pw@cluster0.ab12c.mongodb.net/shop?retryWrites=true&w=majority&authSource=admin",
            hosts,
        )
        .unwrap();
        assert_eq!(atlas.changes.engine, Engine::Mongodb);
        assert_eq!(atlas.changes.host, "cluster0.ab12c.mongodb.net");
        assert_eq!(atlas.changes.database.as_deref(), Some("shop"));
        assert!(atlas.changes.srv);
        assert_eq!(atlas.changes.ssl, Some(ssl(SslMode::VerifyFull)));
        assert_eq!(atlas.ignored, ["retryWrites=true", "w=majority"]);

        let replica =
            parse_connection_url("mongodb://a:27017,b:27018/?tls=true&tlsAllowInvalidCertificates=true", hosts)
                .unwrap();
        assert_eq!((replica.changes.host.as_str(), replica.changes.port.as_deref()), ("a", Some("27017")));
        assert_eq!(replica.changes.ssl, Some(ssl(SslMode::Require)));
        assert_eq!(replica.ignored, ["hosts b:27018"]);
        let mode = |url| parse_connection_url(url, hosts).unwrap().changes.ssl;
        assert_eq!(mode("mongodb://a/?ssl=true"), Some(ssl(SslMode::VerifyFull)));
        assert_eq!(mode("mongodb://a/?tls=true&tlsInsecure=true"), Some(ssl(SslMode::Require)));
        assert_eq!(mode("mongodb://a/"), Some(ssl(SslMode::Disable)));
        let spanish = |hosts: &str| format!("hosts adicionales {hosts}");
        assert_eq!(
            parse_connection_url("mongodb://a:27017,b:27018/", spanish).unwrap().ignored,
            ["hosts adicionales b:27018"]
        );
    }

    #[test]
    fn reads_a_cosmos_connection_string_key_padding_included() {
        let azure = parse_connection_url(
            "AccountEndpoint=https://shop.documents.azure.com:443/;AccountKey=abc+/def==;Database=x;",
            hosts,
        )
        .unwrap();
        assert_eq!(
            azure.changes,
            FormChanges {
                engine: Engine::Cosmosdb,
                host: "https://shop.documents.azure.com/".into(),
                port: None,
                username: String::new(),
                password: "abc+/def==".into(),
                database: None,
                ssl: None,
                srv: false,
            }
        );
        assert_eq!(azure.ignored, ["Database=x"]);
        assert_eq!(parse_connection_url("AccountEndpoint=not a url;AccountKey=k", hosts), None);
        // A bare URI is what the URI field holds; nothing to fill in.
        assert_eq!(parse_connection_url("https://shop.documents.azure.com:443/", hosts), None);
    }

    #[test]
    fn leaves_anything_else_alone() {
        for text in ["localhost", "postgres://x", "https://"] {
            assert_eq!(parse_connection_url(text, hosts), None, "{text}");
        }
    }
}
