//! Queries the user saved with a name, kept in `saved_queries.json`, and the
//! `:name` parameters asked for when one runs (see [`params`]).

pub mod commands;
pub mod params;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::connections::model::Engine;
use crate::error::{AppError, Result};
use crate::files::JsonFile;

/// A query saved with a name, run again from the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    /// The SQL, or the console's script for engines without SQL.
    pub sql: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The folder it is grouped under in the library; none for the top level.
    #[serde(default)]
    pub folder: Option<String>,
    pub scope: SavedScope,
    /// The engine it was written for.
    pub engine: Engine,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    /// The values its parameters were last run with, by parameter name.
    #[serde(default)]
    pub last_values: BTreeMap<String, ParamValue>,
    /// The last [`RECENT_VALUES`] distinct values each parameter ran with,
    /// newest first, by parameter name.
    #[serde(default)]
    pub recent_values: BTreeMap<String, Vec<ParamValue>>,
}

/// How many distinct values each parameter of a saved query remembers.
pub const RECENT_VALUES: usize = 5;

/// Which connections a saved query is offered for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SavedScope {
    /// Only the connection it was saved from.
    #[serde(rename_all = "camelCase")]
    Connection { connection_id: String },
    /// Any connection of the query's [`SavedQuery::engine`].
    Engine,
}

/// The type a parameter's value is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParamKind {
    Text,
    Number,
    Date,
    DateTime,
    Boolean,
    Null,
}

/// A parameter's value as the user typed it, with the type to write it as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamValue {
    pub kind: ParamKind,
    pub value: String,
}

/// What the user enters to create or change a saved query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryInput {
    pub name: String,
    pub sql: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    pub scope: SavedScope,
    pub engine: Engine,
}

/// The saved queries, kept in memory and in their JSON file.
pub struct SavedQueries {
    queries: JsonFile<Vec<SavedQuery>>,
}

impl SavedQueries {
    /// The queries saved at `file`; none until the user saves one.
    pub fn load(file: PathBuf) -> Result<Self> {
        Ok(Self { queries: JsonFile::load(file, Vec::new)? })
    }

    /// Where the file was moved when it couldn't be read at start.
    pub fn set_aside(&self) -> Option<&std::path::Path> {
        self.queries.set_aside()
    }

    /// Every saved query, by folder (top level first) and then by name, ignoring case.
    pub fn list(&self) -> Vec<SavedQuery> {
        let mut queries = self.queries.get();
        queries.sort_by_cached_key(|query| {
            (query.folder.as_deref().map(str::to_lowercase), query.name.to_lowercase(), query.id.clone())
        });
        queries
    }

    pub fn get(&self, id: &str) -> Result<SavedQuery> {
        self.queries
            .read(|queries| queries.iter().find(|query| query.id == id).cloned())
            .ok_or_else(|| AppError::NotFound(id.into()))
    }

    /// Creates a saved query from `input` (`id` is `None`), or changes saved
    /// query `id`, keeping when it was created and last run.
    pub fn save(&self, id: Option<&str>, input: SavedQueryInput) -> Result<SavedQuery> {
        let input = checked(input)?;
        let now = now_ms();
        self.queries.update(|queries| match id {
            None => {
                let query = created(input, new_id(), now);
                queries.push(query.clone());
                Ok(query)
            }
            Some(id) => {
                let existing = find_mut(queries, id)?;
                *existing = SavedQuery {
                    name: input.name,
                    sql: input.sql,
                    description: input.description,
                    folder: input.folder,
                    scope: input.scope,
                    engine: input.engine,
                    updated_at: now,
                    ..existing.clone()
                };
                Ok(existing.clone())
            }
        })
    }

    /// A copy of saved query `id` named "<name> copy", with a new id and no run yet.
    pub fn duplicate(&self, id: &str) -> Result<SavedQuery> {
        let now = now_ms();
        self.queries.update(|queries| {
            let original = find_mut(queries, id)?.clone();
            let copy = SavedQuery {
                id: new_id(),
                name: format!("{} copy", original.name),
                created_at: now,
                updated_at: now,
                last_run_at: None,
                ..original
            };
            queries.push(copy.clone());
            Ok(copy)
        })
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        self.queries.update(|queries| {
            let before = queries.len();
            queries.retain(|query| query.id != id);
            if queries.len() == before {
                return Err(AppError::NotFound(id.into()));
            }
            Ok(())
        })
    }

    /// Records that saved query `id` ran now, remembering `values` when given.
    pub fn mark_run(&self, id: &str, values: Option<BTreeMap<String, ParamValue>>) -> Result<SavedQuery> {
        let now = now_ms();
        self.queries.update(|queries| {
            let query = find_mut(queries, id)?;
            query.last_run_at = Some(now);
            if let Some(values) = values {
                for (name, value) in &values {
                    remember(query.recent_values.entry(name.clone()).or_default(), value.clone());
                }
                query.last_values = values;
            }
            Ok(query.clone())
        })
    }
}

/// Puts `value` first in `recent`, dropping an equal one (same kind and
/// value) and keeping at most [`RECENT_VALUES`].
fn remember(recent: &mut Vec<ParamValue>, value: ParamValue) {
    recent.retain(|kept| *kept != value);
    recent.insert(0, value);
    recent.truncate(RECENT_VALUES);
}

fn find_mut<'a>(queries: &'a mut [SavedQuery], id: &str) -> Result<&'a mut SavedQuery> {
    queries.iter_mut().find(|query| query.id == id).ok_or_else(|| AppError::NotFound(id.into()))
}

/// A new saved query made of `input`, never run.
fn created(input: SavedQueryInput, id: String, now: i64) -> SavedQuery {
    SavedQuery {
        id,
        name: input.name,
        sql: input.sql,
        description: input.description,
        folder: input.folder,
        scope: input.scope,
        engine: input.engine,
        created_at: now,
        updated_at: now,
        last_run_at: None,
        last_values: BTreeMap::new(),
        recent_values: BTreeMap::new(),
    }
}

/// `input` with its name, description and folder trimmed, blank ones left
/// out; a blank name is refused.
fn checked(input: SavedQueryInput) -> Result<SavedQueryInput> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("Enter a name for the saved query.".into()));
    }
    Ok(SavedQueryInput {
        name: name.to_string(),
        description: trimmed(input.description),
        folder: trimmed(input.folder),
        ..input
    })
}

fn trimmed(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_string()).filter(|text| !text.is_empty())
}

/// A new id, made as the other stores make theirs.
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}
