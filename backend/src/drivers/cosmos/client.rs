//! A small client for Cosmos DB's REST API: every request is signed with the
//! account key, recorded in the statement log (see [`logged`](super::logged)),
//! throttled requests are retried when the server says to, and a failed one
//! becomes the server's own message.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use sha2::Sha256;

use super::logged::{self, Request};
use crate::connections::model::Connection;
use crate::connections::transport::SslMode;
use crate::error::{AppError, Result};

/// The REST API version every request asks for.
const API_VERSION: &str = "2018-12-31";
/// How many times a throttled request (429) is sent again.
const RETRIES: u32 = 3;
/// The longest wait before sending a throttled request again.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(5);

/// What a request acts on. Cosmos DB signs a request with the link of the
/// resource itself, or of the parent whose feed of `kind` it reads or adds to.
#[derive(Debug, Clone)]
pub enum Target {
    /// One resource, such as `dbs/shop/colls/orders`; `""` is the account.
    Resource { kind: &'static str, link: String },
    /// The `kind` resources under `parent`, such as the `docs` of a container.
    Feed { kind: &'static str, parent: String },
}

impl Target {
    pub fn account() -> Self {
        Target::Resource { kind: "", link: String::new() }
    }

    pub fn databases() -> Self {
        Target::Feed { kind: "dbs", parent: String::new() }
    }

    pub fn database(database: &str) -> Self {
        Target::Resource { kind: "dbs", link: database_link(database) }
    }

    pub fn containers(database: &str) -> Self {
        Target::Feed { kind: "colls", parent: database_link(database) }
    }

    pub fn container(database: &str, container: &str) -> Self {
        Target::Resource { kind: "colls", link: container_link(database, container) }
    }

    pub fn items(database: &str, container: &str) -> Self {
        Target::Feed { kind: "docs", parent: container_link(database, container) }
    }

    /// The throughput provisioned on the account's databases and containers.
    pub fn offers() -> Self {
        Target::Feed { kind: "offers", parent: String::new() }
    }

    pub fn item(database: &str, container: &str, id: &str) -> Self {
        Target::Resource { kind: "docs", link: format!("{}/docs/{id}", container_link(database, container)) }
    }

    pub fn partition_key_ranges(database: &str, container: &str) -> Self {
        Target::Feed { kind: "pkranges", parent: container_link(database, container) }
    }

    /// The resource's or feed's link as Cosmos DB names it, unencoded:
    /// `dbs/shop/colls/orders`, `dbs/shop/colls`, `dbs`; `""` is the account.
    pub fn link(&self) -> String {
        match self {
            Target::Resource { link, .. } => link.clone(),
            Target::Feed { kind, parent } if parent.is_empty() => (*kind).to_string(),
            Target::Feed { kind, parent } => format!("{parent}/{kind}"),
        }
    }

    /// The database the request is on, if it is on one.
    pub fn database_name(&self) -> Option<&str> {
        let (_, link) = self.signed();
        // Names can't hold `/`, so the database is the link's second segment.
        link.strip_prefix("dbs/").and_then(|rest| rest.split('/').next()).filter(|name| !name.is_empty())
    }

    /// The type and link the request is signed with.
    fn signed(&self) -> (&'static str, &str) {
        match self {
            Target::Resource { kind, link } => (kind, link),
            Target::Feed { kind, parent } => (kind, parent),
        }
    }

    /// The URL path, each name percent-encoded.
    fn path(&self) -> String {
        let encoded = |link: &str| link.split('/').map(encode).collect::<Vec<_>>().join("/");
        match self {
            Target::Resource { link, .. } => format!("/{}", encoded(link)),
            Target::Feed { kind, parent } if parent.is_empty() => format!("/{kind}"),
            Target::Feed { kind, parent } => format!("/{}/{kind}", encoded(parent)),
        }
    }
}

fn database_link(database: &str) -> String {
    format!("dbs/{database}")
}

fn container_link(database: &str, container: &str) -> String {
    format!("dbs/{database}/colls/{container}")
}

/// A reply: its headers (continuation, charge, usage) and its JSON body.
pub struct Reply {
    pub headers: HeaderMap,
    pub body: Value,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

pub struct Client {
    http: reqwest::Client,
    endpoint: String,
    /// Requests go over HTTPS; off only for a local emulator without TLS.
    encrypted: bool,
    key: Vec<u8>,
}

impl Client {
    /// A client for the account at the connection's host and port, over HTTPS
    /// unless TLS is off (the local emulator). The password is the account key.
    pub fn new(connection: &Connection, password: Option<&str>) -> Result<Self> {
        let key = password
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| AppError::Invalid("Enter the account key in Key.".into()))?;
        let key = BASE64.decode(key).map_err(|_| {
            AppError::Invalid("The account key isn't valid: copy the primary key from the account's Keys page.".into())
        })?;
        // reqwest takes rustls' process-wide provider, even for plain HTTP.
        super::super::install_crypto();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .tls_danger_accept_invalid_certs(connection.ssl.mode == SslMode::Require)
            .user_agent("DBForge")
            .build()?;
        let encrypted = connection.ssl.mode.encrypts();
        let scheme = if encrypted { "https" } else { "http" };
        Ok(Self { http, endpoint: format!("{scheme}://{}:{}", connection.host, connection.port), encrypted, key })
    }

    pub async fn get(&self, target: &Target, headers: &[(&str, &str)]) -> Result<Reply> {
        self.send(Method::GET, target, headers, None).await
    }

    /// Sends `body` as JSON, or as a query when `headers` say it is one. The
    /// only way a request reaches the server, so every one is recorded.
    pub async fn send(
        &self,
        method: Method,
        target: &Target,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Reply> {
        let request = Request::new(&method, target, headers, body);
        logged::record(request, self.exchange(&method, target, headers, body)).await
    }

    /// Sends the request until it is answered, again while it is throttled.
    async fn exchange(
        &self,
        method: &Method,
        target: &Target,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Reply> {
        let mut attempt = 0;
        loop {
            let response = self
                .request(method.clone(), target, headers, body)
                .send()
                .await
                .map_err(|error| self.unanswered(error))?;
            let status = response.status();
            let headers = response.headers().clone();
            let text = response.text().await?;
            if status.is_success() {
                let body = if text.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_str(&text)
                        .map_err(|error| AppError::Invalid(format!("The server's reply can't be read: {error}")))?
                };
                return Ok(Reply { headers, body });
            }
            if status == StatusCode::TOO_MANY_REQUESTS && attempt < RETRIES {
                attempt += 1;
                tokio::time::sleep(retry_wait(&headers)).await;
                continue;
            }
            return Err(AppError::Invalid(error_message(status, &text)));
        }
    }

    /// Why a request got no answer. An account only speaks HTTPS, and hangs up
    /// on plain HTTP without a word, which reads as a dropped connection.
    fn unanswered(&self, error: reqwest::Error) -> AppError {
        if self.encrypted || error.is_timeout() || error.is_connect() {
            return error.into();
        }
        AppError::Invalid(
            "The server closed the connection without answering. Cosmos DB accounts are reached over \
             HTTPS: start the URI with https:// (http:// is only for a local emulator without TLS)."
                .into(),
        )
    }

    /// Every resource of a feed, following its continuation tokens; `items` is
    /// the body's list (`Databases`, `DocumentCollections`, …).
    pub async fn list(&self, target: &Target, items: &str) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let mut headers = Vec::new();
            if let Some(token) = &continuation {
                headers.push(("x-ms-continuation", token.as_str()));
            }
            let reply = self.get(target, &headers).await?;
            if let Some(Value::Array(page)) = reply.body.get(items) {
                all.extend(page.iter().cloned());
            }
            continuation = reply.header("x-ms-continuation").map(String::from);
            if continuation.is_none() {
                return Ok(all);
            }
        }
    }

    fn request(
        &self,
        method: Method,
        target: &Target,
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> reqwest::RequestBuilder {
        let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let (kind, link) = target.signed();
        let mut request = self
            .http
            .request(method.clone(), format!("{}{}", self.endpoint, target.path()))
            .header("authorization", self.authorization(&method, kind, link, &date))
            .header("x-ms-date", &date)
            .header("x-ms-version", API_VERSION)
            .header("accept", "application/json");
        let query = is_query(headers);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            let kind = if query { "application/query+json" } else { "application/json" };
            request = request.header("content-type", kind).body(body.to_string());
        }
        request
    }

    /// The master-key signature: an HMAC of the verb, the resource type and
    /// link, and the date, as the REST API documents it.
    fn authorization(&self, method: &Method, kind: &str, link: &str, date: &str) -> String {
        let text = format!("{}\n{kind}\n{link}\n{}\n\n", method.as_str().to_lowercase(), date.to_lowercase());
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC takes keys of any length");
        mac.update(text.as_bytes());
        let signature = BASE64.encode(mac.finalize().into_bytes());
        encode(&format!("type=master&ver=1.0&sig={signature}"))
    }
}

/// Whether the request is a query, which Cosmos DB reads from its headers.
pub(super) fn is_query(headers: &[(&str, &str)]) -> bool {
    headers.iter().any(|(name, _)| *name == "x-ms-documentdb-isquery")
}

/// The wait a throttled reply asks for, kept short.
fn retry_wait(headers: &HeaderMap) -> Duration {
    headers
        .get("x-ms-retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|ms| ms.parse::<f64>().ok())
        .map_or(Duration::from_millis(500), |ms| Duration::from_millis(ms as u64))
        .min(MAX_RETRY_WAIT)
}

/// The server's message without its activity id and request details. Query
/// errors carry a JSON list of errors, which is what the user wants to read.
pub(super) fn error_message(status: StatusCode, body: &str) -> String {
    let reply = serde_json::from_str::<Value>(body).ok();
    let errors = |details: &Value| Some(query_errors(details)).filter(|errors| !errors.is_empty());
    let message = reply.as_ref().and_then(errors).unwrap_or_else(|| {
        let message = reply
            .as_ref()
            .and_then(|reply| reply.get("message").and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| body.trim().to_string());
        let first = message.split(['\r', '\n']).next().unwrap_or_default().trim();
        let first = first.strip_prefix("Message: ").unwrap_or(first);
        serde_json::from_str::<Value>(first)
            .ok()
            .as_ref()
            .and_then(errors)
            .unwrap_or_else(|| first.trim_end_matches(',').to_string())
    });
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN if message.contains("authorization token") => {
            "Authentication failed: the account key is wrong. Copy the primary key from the account's Keys page.".into()
        }
        StatusCode::PRECONDITION_FAILED => {
            "The item was changed by someone else since it was loaded, so it was left as it is.".into()
        }
        StatusCode::CONFLICT => "An item with this id already exists in this partition.".into(),
        _ if message.is_empty() => format!("The server answered {status}"),
        _ => message,
    }
}

/// The messages of `{"errors": [{"message": …}]}` or `{"Errors": ["…"]}`.
fn query_errors(details: &Value) -> String {
    let errors = details.get("errors").or_else(|| details.get("Errors")).and_then(Value::as_array);
    errors
        .map(|errors| {
            errors
                .iter()
                .filter_map(|error| error.get("message").and_then(Value::as_str).or_else(|| error.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Percent-encodes everything but the characters a URL never needs escaped.
fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that accepts one connection and closes it without answering,
    /// as an account does when spoken to in plain HTTP.
    async fn hanging_up() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request).await;
        });
        port
    }

    #[tokio::test]
    async fn a_dropped_plain_http_request_points_at_https() {
        let port = hanging_up().await;
        let connection = Connection { port, ..crate::drivers::cosmos::tests::local() };
        let client = Client::new(&connection, crate::drivers::cosmos::tests::KEY).unwrap();
        let Err(error) = client.get(&Target::account(), &[]).await else { panic!("the request was answered") };
        let error = error.to_string();
        assert!(error.contains("start the URI with https://"), "{error}");
    }

    #[tokio::test]
    async fn an_unreachable_server_keeps_its_own_message() {
        let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let connection = Connection { port, ..crate::drivers::cosmos::tests::local() };
        let client = Client::new(&connection, crate::drivers::cosmos::tests::KEY).unwrap();
        let Err(error) = client.get(&Target::account(), &[]).await else { panic!("the request was answered") };
        let error = error.to_string();
        assert!(error.starts_with("Could not reach the server"), "{error}");
    }

    #[test]
    fn paths_encode_names_and_sign_with_the_unencoded_link() {
        // Names can't hold `/`, `\\`, `?` or `#`; anything else is allowed.
        let container = Target::container("shop", "big orders+é");
        assert_eq!(container.path(), "/dbs/shop/colls/big%20orders%2B%C3%A9");
        assert_eq!(container.signed(), ("colls", "dbs/shop/colls/big orders+é"));
        assert_eq!(Target::databases().path(), "/dbs");
        assert_eq!(Target::items("shop", "orders").path(), "/dbs/shop/colls/orders/docs");
        assert_eq!(Target::items("shop", "orders").signed(), ("docs", "dbs/shop/colls/orders"));
        assert_eq!(Target::account().path(), "/");
    }

    #[test]
    fn errors_read_as_the_server_wrote_them() {
        let query = r#"{"code":"BadRequest","message":"Message: {\"errors\":[{\"severity\":\"Error\",\"location\":{\"start\":9,\"end\":13},\"code\":\"SC2001\",\"message\":\"Identifier 'nope' could not be resolved.\"}]}\r\nActivityId: 1, Microsoft.Azure.Documents.Common/2.14.0"}"#;
        assert_eq!(error_message(StatusCode::BAD_REQUEST, query), "Identifier 'nope' could not be resolved.");

        let missing = r#"{"code":"NotFound","message":"Entity with the specified id does not exist in the system. More info: https://aka.ms/cosmosdb-tsg-not-found, \r\nRequestStartTime: 2026-11-09"}"#;
        assert_eq!(
            error_message(StatusCode::NOT_FOUND, missing),
            "Entity with the specified id does not exist in the system. More info: https://aka.ms/cosmosdb-tsg-not-found"
        );

        let bare = r#"{"errors":[{"severity":"Error","code":"SC1001","message":"Syntax error, incorrect syntax near 'FORM'."}]}"#;
        assert_eq!(error_message(StatusCode::BAD_REQUEST, bare), "Syntax error, incorrect syntax near 'FORM'.");

        let key = r#"{"code":"Unauthorized","message":"The input authorization token can't serve the request."}"#;
        assert!(error_message(StatusCode::UNAUTHORIZED, key).starts_with("Authentication failed"));
        assert_eq!(error_message(StatusCode::BAD_GATEWAY, ""), "The server answered 502 Bad Gateway");
        assert!(error_message(StatusCode::PRECONDITION_FAILED, "{}").starts_with("The item was changed"));
        assert!(error_message(StatusCode::CONFLICT, "{}").starts_with("An item with this id already exists"));
    }
}
