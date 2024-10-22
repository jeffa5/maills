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
