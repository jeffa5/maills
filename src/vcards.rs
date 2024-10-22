use std::{
    collections::BTreeMap,
    fs::{read_dir, read_to_string, File},
    io::Write,
    path::PathBuf,
};

use itertools::Itertools as _;
use uriparse::URI;
use vcard4::{property::Property as _, Vcard, VcardBuilder};

use crate::{ContactSource, Location, Mailbox};

pub struct VCards {
    root: PathBuf,
    vcards: BTreeMap<PathBuf, Vec<vcard4::Vcard>>,
}

impl ContactSource for VCards {
    fn render(&self, mailbox: &Mailbox) -> String {
        let vcards = self.get_by_mailbox(mailbox);
        vcards
            .iter()
            .map(|vc| render_vcard(vc))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn find_matching(&self, word: &str) -> Vec<Mailbox> {
        self.vcards
            .values()
            .flatten()
            .filter(|vc| match_vcard(vc, word))
            .flat_map(mailboxes_for_vcard)
            .unique()
            .collect()
    }

    fn contains(&self, email: &str) -> bool {
        self.vcards.values().flatten().any(|vc| {
            vc.email
                .iter()
                .any(|e| e.value.to_lowercase() == email.to_lowercase())
        })
    }

    fn locations(&self, mailbox: &Mailbox) -> Vec<Location> {
        self.vcards
            .iter()
            .filter(|(_, vcs)| {
                vcs.iter().any(|vc| {
                    vc.email
                        .iter()
                        .any(|e| e.value.to_lowercase() == mailbox.email.to_lowercase())
                        && mailbox.name.as_ref().map_or(true, |name| {
                            vc.formatted_name
                                .iter()
                                .any(|f| f.value.to_lowercase() == name.to_lowercase())
                        })
                })
            })
            .map(|(p, _)| Location {
                path: p.clone(),
                line: None,
            })
            .collect()
    }

    fn create_contact(&mut self, mailbox: Mailbox) -> Option<PathBuf> {
        let filename = uuid::Uuid::new_v4().to_string();
        let path = self.root.join(&filename).with_extension("vcf");
        let vcard = VcardBuilder::new(mailbox.name.unwrap_or_default())
            .uid(
                URI::try_from(format!("urn:uuid:{}", filename).as_str())
                    .unwrap()
                    .into_owned(),
            )
            .email(mailbox.email)
            .finish();
        let mut f = File::create(&path).unwrap();
        f.write_all(vcard.to_string().as_bytes()).unwrap();
        self.vcards.insert(path.clone(), vec![vcard]);
        Some(path)
    }
}

impl VCards {
    pub fn new(value: PathBuf) -> Self {
        let mut s = Self {
            root: value,
            vcards: BTreeMap::new(),
        };
        s.load_vcards();
        s
    }

    fn load_vcards(&mut self) {
        let mut vcard_files = Vec::new();
        for entry in read_dir(&self.root).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() && path.extension().unwrap_or_default() == "vcf" {
                vcard_files.push(path);
            }
        }

        self.vcards.clear();
        for path in vcard_files {
            let content = read_to_string(&path).unwrap_or_default();
            match vcard4::parse_loose(content) {
                Ok(vcards) => self.vcards.entry(path).or_default().extend(vcards),
                Err(err) => {
                    // skip card that couldn't be loaded
                    eprintln!("Failed to load vcard at {:?}: {}", path, err);
                }
            }
        }
    }

    fn get_by_mailbox(&self, mailbox: &Mailbox) -> Vec<&Vcard> {
        self.vcards
            .values()
            .flatten()
            .filter(|vc| {
                vc.email
                    .iter()
                    .any(|e| e.value.to_lowercase() == mailbox.email.to_lowercase())
                    && mailbox.name.as_ref().map_or(true, |name| {
                        vc.formatted_name
                            .iter()
                            .any(|f| f.value.to_lowercase() == name.to_lowercase())
                    })
            })
            .collect()
    }
}

fn render_vcard(vcard: &Vcard) -> String {
    let mut lines = Vec::new();
    if let Some(formatted_name) = vcard.formatted_name.first() {
        lines.push(format!("# {}", formatted_name.value));
        lines.push(String::new());
    }
    if let Some(nick) = vcard.nickname.first() {
        lines.push(format!("_{}_", nick.value));
        lines.push(String::new());
    }
    if !vcard.email.is_empty() {
        lines.push("Email:".to_owned());
        for e in vcard.email.iter() {
            let mut line = "- ".to_owned();
            if let Some(typ) = &e
                .parameters()
                .and_then(|p| p.types.as_ref().and_then(|types| types.first()))
            {
                line.push_str(&typ.to_string());
                line.push_str(": ");
            }
            line.push_str(&e.value);
            lines.push(line);
        }
        lines.push(String::new());
    }
    if !vcard.tel.is_empty() {
        lines.push("Telephone:".to_owned());
        for e in vcard.tel.iter() {
            let mut line = "- ".to_owned();
            if let Some(typ) = &e
                .parameters()
                .and_then(|p| p.types.as_ref().and_then(|types| types.first()))
            {
                line.push_str(&typ.to_string());
                line.push_str(": ");
            }
            line.push_str(&e.to_string());
            lines.push(line);
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

fn match_vcard(vc: &Vcard, word: &str) -> bool {
    let matched_email = vc
        .email
        .iter()
        .any(|e| e.value.to_lowercase().contains(word));
    let matched_fn = vc
        .formatted_name
        .iter()
        .any(|n| n.value.to_lowercase().contains(word));
    let matched_nick = vc
        .nickname
        .iter()
        .any(|n| n.value.to_lowercase().contains(word));
    matched_email || matched_fn || matched_nick
}

fn mailboxes_for_vcard(vcard: &Vcard) -> Vec<Mailbox> {
    let formatted_name = vcard.formatted_name.first().map(|n| &n.value);
    vcard
        .email
        .iter()
        .map(|e| {
            if let Some(n) = formatted_name {
                Mailbox {
                    name: Some(n.to_owned()),
                    email: e.value.clone(),
                }
            } else {
                Mailbox {
                    name: None,
                    email: e.value.clone(),
                }
            }
        })
        .collect()
}
