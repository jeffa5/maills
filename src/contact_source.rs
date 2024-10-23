use std::path::PathBuf;

use itertools::Itertools as _;
use lsp_types::Url;

use crate::Mailbox;

pub trait ContactSource {
    /// Render a version of the contact for this mailbox using markdown.
    fn render(&self, mailbox: &Mailbox) -> String;

    /// Find any matching mailboxes.
    fn find_matching(&self, word: &str) -> Vec<Mailbox>;

    /// Whether the given mailbox is in the source.
    fn contains(&self, email: &str) -> bool;

    /// Get the locations for the given mailbox.
    fn locations(&self, mailbox: &Mailbox) -> Vec<Location>;

    /// Create the contact for the given mailbox, returning the path to it.
    fn create_contact(&mut self, mailbox: Mailbox) -> Option<PathBuf>;
}

#[derive(Default)]
pub struct Sources {
    pub sources: Vec<Box<dyn ContactSource>>,
}

impl ContactSource for Sources {
    fn render(&self, mailbox: &Mailbox) -> String {
        self.sources
            .iter()
            .map(|s| s.render(mailbox))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn find_matching(&self, word: &str) -> Vec<Mailbox> {
        self.sources
            .iter()
            .flat_map(|s| s.find_matching(word))
            .unique()
            .collect()
    }

    fn contains(&self, email: &str) -> bool {
        self.sources.iter().any(|s| s.contains(email))
    }

    fn locations(&self, mailbox: &Mailbox) -> Vec<Location> {
        self.sources
            .iter()
            .flat_map(|s| s.locations(mailbox))
            .collect()
    }

    fn create_contact(&mut self, mailbox: Mailbox) -> Option<PathBuf> {
        for s in &mut self.sources {
            if let Some(path) = s.create_contact(mailbox.clone()) {
                return Some(path);
            }
        }
        None
    }
}

pub struct Location {
    pub path: PathBuf,
    pub line: Option<u32>,
}

impl From<Location> for lsp_types::Location {
    fn from(value: Location) -> Self {
        lsp_types::Location {
            uri: Url::from_file_path(value.path).unwrap(),
            range: if let Some(line) = value.line {
                lsp_types::Range {
                    start: lsp_types::Position { line, character: 0 },
                    end: lsp_types::Position { line, character: 0 },
                }
            } else {
                lsp_types::Range::default()
            },
        }
    }
}
