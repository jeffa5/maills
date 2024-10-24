use std::{
    collections::{HashMap, HashSet},
    fs::read_to_string,
    path::PathBuf,
};

use crate::{ContactSource, Location, Mailbox};

struct ContactListEntry {
    mailbox: Mailbox,
    line: u32,
}

pub struct ContactList {
    path: PathBuf,
    diagnostics: bool,
    contacts: Vec<ContactListEntry>,
    contact_lines: HashMap<Mailbox, usize>,
    emails_lower: HashSet<String>,
}

impl ContactSource for ContactList {
    fn render(&self, mailbox: &Mailbox) -> String {
        let mut lines = Vec::new();
        if let Some(name) = &mailbox.name {
            lines.push(format!("# {}", name));
            lines.push(String::new());
        }
        lines.push("Email:".to_owned());
        lines.push(format!("- {}", mailbox.email));
        lines.join("\n")
    }

    fn find_matching(&self, word: &str) -> Vec<Mailbox> {
        self.contacts
            .iter()
            .map(|e| &e.mailbox)
            .filter(|m| {
                // TODO: make this contains check cheaper, rather than searching every entry
                // Likely a custom trie
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
        if self.diagnostics {
            self.emails_lower.contains(&email.to_lowercase())
        } else {
            // contains nothing with respec to diagnostics
            false
        }
    }

    fn locations(&self, mailbox: &Mailbox) -> Vec<Location> {
        let line = self
            .contact_lines
            .get(mailbox)
            .map(|i| &self.contacts[*i].line)
            .copied();
        vec![Location {
            path: self.path.clone(),
            line,
        }]
    }

    fn create_contact(&mut self, _mailbox: Mailbox) -> Option<PathBuf> {
        // not supported
        None
    }
}

impl ContactList {
    pub fn new(path: PathBuf, diagnostics: bool) -> Self {
        let mut s = Self {
            path,
            diagnostics,
            contacts: Vec::new(),
            contact_lines: HashMap::new(),
            emails_lower: HashSet::new(),
        };
        s.load_contactlist();
        s
    }

    fn load_contactlist(&mut self) {
        let content = read_to_string(&self.path).unwrap();
        for (line_number, line) in content.lines().enumerate() {
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
            self.emails_lower.insert(email.to_lowercase());
            let mbox = Mailbox { name, email };
            self.contact_lines.insert(mbox.clone(), self.contacts.len());
            self.contacts.push(ContactListEntry {
                mailbox: mbox,
                line: line_number as u32,
            });
        }
    }
}
