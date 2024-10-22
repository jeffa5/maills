use std::path::PathBuf;

use crate::Mailbox;

pub trait ContactSource {
    /// Render a version of the contact for this mailbox using markdown.
    fn render(&self, mailbox: &Mailbox) -> String;

    /// Find any matching mailboxes.
    fn find_matching(&self, word: &str) -> Vec<Mailbox>;

    /// Whether the given mailbox is in the source.
    fn contains(&self, mailbox: &Mailbox) -> bool;

    /// Get the filepaths for the given mailbox.
    fn filepaths(&self, mailbox: &Mailbox) -> Vec<PathBuf>;

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
            .collect()
    }

    fn contains(&self, mailbox: &Mailbox) -> bool {
        self.sources.iter().any(|s| s.contains(mailbox))
    }

    fn filepaths(&self, mailbox: &Mailbox) -> Vec<PathBuf> {
        self.sources
            .iter()
            .flat_map(|s| s.filepaths(mailbox))
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
