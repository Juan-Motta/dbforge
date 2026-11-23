//! DBForge's guide for agents, served by the MCP server itself so an agent
//! learns how DBForge works without fetching a skill from elsewhere: the
//! `dbforge_guide` tool and the `dbforge://guide` resources give the same
//! text. It is static, so the stdio bridge serves it too, with the app closed.
//!
//! Each section is a Markdown file beside this one, written to stand alone.

use rmcp::ErrorData;
use rmcp::model::{ListResourcesResult, ReadResourceResult, Resource, ResourceContents};

/// The URI of the guide's start: the overview and the list of sections.
pub const URI: &str = "dbforge://guide";
const MIME_TYPE: &str = "text/markdown";

/// One part of the guide.
pub struct Section {
    /// What `dbforge_guide` takes as `section`, and the end of its resource's URI.
    pub name: &'static str,
    /// One line on what it covers, for the list of sections.
    pub summary: &'static str,
    pub text: &'static str,
}

pub const SECTIONS: &[Section] = &[
    Section {
        name: "overview",
        summary: "What DBForge's MCP server is, the rules every agent follows, the tools, and how to read errors.",
        text: include_str!("guide/overview.md"),
    },
    Section {
        name: "queries",
        summary: "query vs execute, how each engine keeps query read-only, row limits, results and each engine's language.",
        text: include_str!("guide/queries.md"),
    },
    Section {
        name: "saved-queries",
        summary: "Listing, running, saving and deleting the user's saved queries; :name parameters and their values.",
        text: include_str!("guide/saved-queries.md"),
    },
    Section {
        name: "backups",
        summary: "Backing up and restoring whole databases: options and defaults, file names, and what a restore does.",
        text: include_str!("guide/backups.md"),
    },
    Section {
        name: "connections",
        summary: "Reading a connection's settings and creating, changing or deleting connections without handling secrets.",
        text: include_str!("guide/connections.md"),
    },
    Section {
        name: "engines",
        summary: "Notes per engine: PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, MongoDB, Cosmos DB and Redis.",
        text: include_str!("guide/engines.md"),
    },
];

/// The section named `name`.
fn section(name: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|section| section.name == name)
}

/// The guide's start: the overview, then every section with its summary.
fn start() -> String {
    let mut text = SECTIONS[0].text.to_owned();
    text.push_str("\n## Sections\n\nRead one with `dbforge_guide` and its `section`, or the `dbforge://guide/<section>` resource.\n\n");
    for section in SECTIONS {
        text.push_str(&format!("- `{}`: {}\n", section.name, section.summary));
    }
    text
}

/// Every section's name, for a refusal.
fn names() -> String {
    SECTIONS.iter().map(|section| section.name).collect::<Vec<_>>().join(", ")
}

/// What `dbforge_guide` answers: the start with no section, or the section
/// named; an unknown name, with the names there are.
pub fn read(name: Option<&str>) -> Result<String, String> {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        None => Ok(start()),
        Some(name) => section(name)
            .map(|section| section.text.to_owned())
            .ok_or_else(|| format!("There's no section {name} in DBForge's guide. The sections: {}.", names())),
    }
}

/// The guide's resources: its start, then each section.
pub fn resources() -> ListResourcesResult {
    let start = Resource::new(URI, "guide")
        .with_description("DBForge's guide for agents: the rules, then the list of sections.")
        .with_mime_type(MIME_TYPE);
    let sections = SECTIONS.iter().map(|section| {
        Resource::new(format!("{URI}/{}", section.name), format!("guide/{}", section.name))
            .with_description(section.summary)
            .with_mime_type(MIME_TYPE)
    });
    ListResourcesResult::with_all_items(std::iter::once(start).chain(sections).collect())
}

/// The resource at `uri`, as `text/markdown`.
pub fn resource(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let text = match uri.strip_prefix(URI) {
        Some("") => Ok(start()),
        Some(rest) => match rest.strip_prefix('/').and_then(section) {
            Some(section) => Ok(section.text.to_owned()),
            None => Err(()),
        },
        None => Err(()),
    }
    .map_err(|()| {
        let sections = SECTIONS.iter().map(|section| format!("{URI}/{}", section.name)).collect::<Vec<_>>();
        ErrorData::resource_not_found(
            format!("No resource {uri}. DBForge serves {URI} and {}.", sections.join(", ")),
            None,
        )
    })?;
    Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri).with_mime_type(MIME_TYPE)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::Tools;

    const NAMES: [&str; 6] = ["overview", "queries", "saved-queries", "backups", "connections", "engines"];

    fn text_of(result: &ReadResourceResult) -> (&str, Option<&str>) {
        match result.contents.as_slice() {
            [ResourceContents::TextResourceContents { text, mime_type, .. }] => (text, mime_type.as_deref()),
            other => panic!("expected one text, got {other:?}"),
        }
    }

    #[test]
    fn every_section_reads_on_its_own() {
        assert_eq!(SECTIONS.iter().map(|section| section.name).collect::<Vec<_>>(), NAMES);
        for name in NAMES {
            let text = read(Some(name)).unwrap();
            assert_eq!(text, section(name).unwrap().text);
            assert!(text.starts_with("# "), "{name} starts with its title");
        }
    }

    #[test]
    fn no_section_gives_the_overview_and_every_section_with_its_summary() {
        let text = read(None).unwrap();
        assert!(text.starts_with(section("overview").unwrap().text));
        for section in SECTIONS {
            assert!(!section.summary.is_empty());
            assert!(text.contains(&format!("- `{}`: {}", section.name, section.summary)), "{} listed", section.name);
        }
    }

    #[test]
    fn an_unknown_section_names_the_ones_there_are() {
        let error = read(Some("nope")).unwrap_err();
        assert!(error.contains("nope"));
        for name in NAMES {
            assert!(error.contains(name), "{name} named");
        }
    }

    #[test]
    fn resources_give_the_same_text_as_markdown() {
        let listed = resources().resources;
        let uris: Vec<&str> = listed.iter().map(|resource| resource.uri.as_str()).collect();
        let mut expected = vec![URI.to_owned()];
        expected.extend(NAMES.iter().map(|name| format!("{URI}/{name}")));
        assert_eq!(uris, expected);
        assert!(listed.iter().all(|resource| resource.mime_type.as_deref() == Some(MIME_TYPE)));

        let start = resource(URI).unwrap();
        assert_eq!(text_of(&start), (read(None).unwrap().as_str(), Some(MIME_TYPE)));
        for name in NAMES {
            let read_back = resource(&format!("{URI}/{name}")).unwrap();
            assert_eq!(text_of(&read_back), (read(Some(name)).unwrap().as_str(), Some(MIME_TYPE)));
        }
        assert!(resource("dbforge://guide/nope").is_err());
        assert!(resource("file:///etc/passwd").is_err());
    }

    #[test]
    fn the_guide_mentions_every_tool() {
        let guide: String = SECTIONS.iter().map(|section| section.text).collect();
        for tool in Tools::tool_router().list_all() {
            assert!(guide.contains(tool.name.as_ref()), "the guide never mentions {}", tool.name);
        }
    }
}
