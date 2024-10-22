use std::{fs::read_to_string, path::PathBuf};

use crate::Mailbox;

pub struct ContactList {
    path: PathBuf,
    contacts: Vec<Mailbox>,
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
