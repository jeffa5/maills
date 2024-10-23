use std::{fmt::Display, str::FromStr, sync::LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

static MAILBOX_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    regex::Regex::new(
            r#"(?i)(?<name>("[\w \-']+"|[\w \-']+))?\s*<?\b(?<email>[A-Z0-9._%+-~/]+@[A-Z0-9.-]+\.[A-Z]{2,})\b>?"#,
        )
        .unwrap()
});

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: Option<String>,
    pub email: String,
}

impl Mailbox {
    pub fn from_line_at(line: &str, character: usize) -> Option<Self> {
        let mut mailbox = None;
        for captures in MAILBOX_REGEX.captures_iter(line) {
            let mut start = None;
            let mut end = None;
            let mut mbox = Mailbox::default();

            if let Some(name) = captures.name("name") {
                start = Some(name.start());
                end = Some(name.end());
                mbox.name = Some(
                    name.as_str()
                        .trim()
                        .trim_start_matches('"')
                        .trim_end_matches('"')
                        .to_owned(),
                );
            }
            if let Some(email) = captures.name("email") {
                if start.is_none() {
                    start = Some(email.start());
                }
                end = Some(email.end());
                mbox.email = email.as_str().trim().to_owned();
            }

            if start.map_or(false, |s| s <= character) && end.map_or(false, |e| character <= e) {
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
        if let Some((name, email)) = s.split_once(" <") {
            let name = name
                .trim()
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim()
                .to_owned();
            let email = email.trim().trim_end_matches('>').to_owned();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str() {
        let s = "First Last <first.last@test.com>";
        let mbox = Mailbox::from_str(s).unwrap();
        assert_eq!(
            mbox,
            Mailbox {
                name: Some("First Last".to_owned()),
                email: "first.last@test.com".to_owned(),
            }
        );
        assert_eq!(Mailbox::from_str(&mbox.to_string()).unwrap(), mbox);
    }

    #[test]
    fn from_line_at() {
        let line = "First Last <first.last@test.com>";
        let expected = Some(Mailbox {
            name: Some("First Last".to_owned()),
            email: "first.last@test.com".to_owned(),
        });
        for i in 0..line.len() {
            assert_eq!(
                Mailbox::from_line_at(line, i),
                expected,
                "character {} {:?}",
                i,
                line.chars().nth(i)
            );
        }
    }

    #[test]
    fn from_line_at_quote() {
        let line = "\"First Last\" <first.last@test.com>";
        let expected = Some(Mailbox {
            name: Some("First Last".to_owned()),
            email: "first.last@test.com".to_owned(),
        });
        for i in 0..line.len() {
            assert_eq!(
                Mailbox::from_line_at(line, i),
                expected,
                "character {} {:?}",
                i,
                line.chars().nth(i)
            );
        }
    }

    #[test]
    fn from_line_at_context() {
        let line = "Other words before \"First Last\" <first.last@test.com> and other words after";
        let expected = Some(Mailbox {
            name: Some("First Last".to_owned()),
            email: "first.last@test.com".to_owned(),
        });
        for i in 19..53 {
            assert_eq!(
                Mailbox::from_line_at(line, i),
                expected,
                "character {} {:?}",
                i,
                line.chars().nth(i)
            );
        }
    }
}
