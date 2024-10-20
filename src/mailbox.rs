use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: Option<String>,
    pub email: String,
}

impl Mailbox {
    pub fn from_line_at(line: &str, character: usize) -> Option<Self> {
        let re = regex::Regex::new(
            r#"(?i)"?(?<name>[\w \-']+)?"? ?<?\b(?<email>[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,})\b>?"#,
        )
        .unwrap();
        let mut mailbox = None;
        for captures in re.captures_iter(line) {
            let mut start = None;
            let mut end = None;
            let mut mbox = Mailbox::default();

            if let Some(name) = captures.name("name") {
                start = Some(name.start());
                end = Some(name.end());
                mbox.name = Some(name.as_str().trim().to_owned());
            }
            if let Some(email) = captures.name("email") {
                if start.is_none() {
                    start = Some(email.start());
                }
                end = Some(email.end());
                mbox.email = email.as_str().trim().to_owned();
            }

            if start.map_or(false, |s| s <= character) && end.map_or(false, |e| character < e) {
                mailbox = Some(mbox);
                break;
            }
        }
        mailbox
    }
}

impl FromStr for Mailbox {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((name, email)) = s.split_once("\" <") {
            let name = name.trim_start_matches('"').to_owned();
            let email = email.trim_end_matches('>').to_owned();
            Ok(Self {
                name: Some(name),
                email,
            })
        } else {
            Ok(Self {
                name: None,
                email: s.to_owned(),
            })
        }
    }
}

impl Display for Mailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = &self.name {
            write!(f, "{:?} <", name)?;
        }
        write!(f, "{}", self.email)?;
        if self.name.is_some() {
            write!(f, ">")?;
        }
        Ok(())
    }
}
