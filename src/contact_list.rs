use std::{fs::read_to_string, path::PathBuf};

use crate::{ContactSource, Mailbox};

pub struct ContactList {
    path: PathBuf,
    contacts: Vec<Mailbox>,
}

impl ContactSource for ContactList {
    fn render(&self, mailbox: &Mailbox) -> String {
        mailbox.to_string()
    }

    fn find_matching(&self, word: &str) -> Vec<Mailbox> {
        self.contacts
            .iter()
            .filter(|m| {
                let matched_name = m
                    .name
                    .as_ref()
                    .map_or(false, |n| n.to_lowercase().contains(word));
                let matched_email = m.email.to_lowercase().contains(word);
                matched_name || matched_email
            })
            .cloned()
            .collect()
    }

    fn contains(&self, email: &str) -> bool {
        self.contacts
            .iter()
            .any(|m| m.email.to_lowercase() == email.to_lowercase())
    }

    fn filepaths(&self, _mailbox: &Mailbox) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }

    fn create_contact(&mut self, _mailbox: Mailbox) -> Option<PathBuf> {
        // not supported
        None
    }
}

impl ContactList {
    pub fn new(path: PathBuf) -> Self {
        let mut s = Self {
            path,
            contacts: Vec::new(),
        };
        s.load_contactlist();
        s
    }

    fn load_contactlist(&mut self) {
        let content = read_to_string(&self.path).unwrap();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split(' ').collect::<Vec<_>>();
            let email = parts.remove(parts.len() - 1).to_owned();
            let name = if !parts.is_empty() {
                Some(parts.join(" "))
            } else {
                None
            };
            let mbox = Mailbox { name, email };
            self.contacts.push(mbox);
        }
    }
}
