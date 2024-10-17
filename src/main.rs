use clap::Parser;
use lsp_server::ErrorCode;
use lsp_server::Message;
use lsp_server::Notification;
use lsp_server::Request;
use lsp_server::RequestId;
use lsp_server::Response;
use lsp_server::ResponseError;
use lsp_server::{Connection, IoThreads};
use lsp_types::notification::LogMessage;
use lsp_types::notification::Notification as _;
use lsp_types::notification::ShowMessage;
use lsp_types::request::Request as _;
use lsp_types::CompletionItem;
use lsp_types::CompletionList;
use lsp_types::ExecuteCommandOptions;
use lsp_types::InitializeParams;
use lsp_types::InitializeResult;
use lsp_types::Location;
use lsp_types::Position;
use lsp_types::PositionEncodingKind;
use lsp_types::Range;
use lsp_types::ServerCapabilities;
use lsp_types::ServerInfo;
use lsp_types::ShowDocumentParams;
use lsp_types::TextDocumentPositionParams;
use lsp_types::TextDocumentSyncKind;
use lsp_types::Url;
use serde::Deserialize;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
struct Args {
    #[clap(long)]
    stdio: bool,
}

fn log(c: &Connection, message: impl Serialize) {
    c.sender
        .send(Message::Notification(Notification::new(
            LogMessage::METHOD.to_string(),
            message,
        )))
        .unwrap();
}

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        completion_provider: Some(lsp_types::CompletionOptions {
            resolve_provider: Some(true),
            ..Default::default()
        }),
        text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Options(
            lsp_types::TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                ..Default::default()
            },
        )),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec!["define".to_owned()],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn connect(stdio: bool) -> (lsp_types::InitializeParams, Connection, IoThreads) {
    let (connection, io) = if stdio {
        Connection::stdio()
    } else {
        panic!("No connection mode given, e.g. --stdio");
    };
    let (id, params) = connection.initialize_start().unwrap();
    let mut caps = server_capabilities();
    let init_params = serde_json::from_value::<InitializeParams>(params).unwrap();
    if let Some(general) = &init_params.capabilities.general {
        let pe = general
            .position_encodings
            .clone()
            .unwrap_or_default()
            .iter()
            .find(|&pe| *pe == PositionEncodingKind::UTF8)
            .cloned()
            .unwrap_or(PositionEncodingKind::UTF16);
        caps.position_encoding = Some(pe);
    }
    let init_opts = if let Some(io) = &init_params.initialization_options {
        match serde_json::from_value::<InitializationOptions>(io.clone()) {
            Ok(v) => v,
            Err(err) => {
                connection
                    .sender
                    .send(Message::Notification(Notification::new(
                        ShowMessage::METHOD.to_string(),
                        format!("Invalid initialization options: {err}"),
                    )))
                    .unwrap();
                panic!("Invalid initialization options: {err}")
            }
        }
    } else {
        connection
            .sender
            .send(Message::Notification(Notification::new(
                ShowMessage::METHOD.to_string(),
                "No initialization options given, need it for vcard directory location at least"
                    .to_string(),
            )))
            .unwrap();
        panic!("No initialization options given, need it for vcard directory location at least")
    };
    if !init_opts.enable_completion.unwrap_or(true) {
        caps.completion_provider = None;
    }
    if !init_opts.enable_hover.unwrap_or(true) {
        caps.hover_provider = None;
    }
    if !init_opts.enable_code_actions.unwrap_or(true) {
        caps.code_action_provider = None;
        caps.execute_command_provider = None;
    }
    if !init_opts.enable_goto_definition.unwrap_or(true) {
        caps.definition_provider = None;
    }
    let init_result = InitializeResult {
        capabilities: caps,
        server_info: Some(ServerInfo {
            name: "maills".to_owned(),
            version: None,
        }),
    };
    connection
        .initialize_finish(id, serde_json::to_value(init_result).unwrap())
        .unwrap();
    // log(&c, format!("{:?}", params.initialization_options));
    (init_params, connection, io)
}

struct Server {
    vcards: VCards,
    open_files: BTreeMap<String, String>,
    shutdown: bool,
}

#[derive(Serialize, Deserialize)]
struct InitializationOptions {
    vcard_dir: PathBuf,
    enable_completion: Option<bool>,
    enable_hover: Option<bool>,
    enable_code_actions: Option<bool>,
    enable_goto_definition: Option<bool>,
}

impl Server {
    fn new(c: &Connection, params: lsp_types::InitializeParams) -> Self {
        let init_opts = if let Some(io) = params.initialization_options {
            match serde_json::from_value::<InitializationOptions>(io) {
                Ok(v) => v,
                Err(err) => {
                    c.sender
                        .send(Message::Notification(Notification::new(
                            ShowMessage::METHOD.to_string(),
                            format!("Invalid initialization options: {err}"),
                        )))
                        .unwrap();
                    panic!("Invalid initialization options: {err}")
                }
            }
        } else {
            c.sender
                .send(Message::Notification(Notification::new(
                    ShowMessage::METHOD.to_string(),
                    "No initialization options given, need it for vcard directory location at least"
                        .to_string(),
                )))
                .unwrap();
            panic!("No initialization options given, need it for vcard directory location at least")
        };
        let vcard_root = if init_opts.vcard_dir.starts_with("~/") {
            dirs::home_dir()
                .unwrap()
                .join(init_opts.vcard_dir.strip_prefix("~/").unwrap())
        } else {
            init_opts.vcard_dir
        };
        Self {
            vcards: VCards::new(&vcard_root),
            open_files: BTreeMap::new(),
            shutdown: false,
        }
    }

    fn serve(mut self, c: Connection) -> Result<(), String> {
        loop {
            match c.receiver.recv().unwrap() {
                Message::Request(r) => {
                    // log(&c, format!("Got request {r:?}"));
                    if self.shutdown {
                        c.sender
                            .send(Message::Response(Response {
                                id: r.id,
                                result: None,
                                error: Some(ResponseError {
                                    code: ErrorCode::InvalidRequest as i32,
                                    message: String::from("received request after shutdown"),
                                    data: None,
                                }),
                            }))
                            .unwrap();
                        continue;
                    }

                    match &r.method[..] {
                        lsp_types::request::HoverRequest::METHOD => {
                            let tdp =
                                serde_json::from_value::<lsp_types::TextDocumentPositionParams>(
                                    r.params,
                                )
                                .unwrap();

                            let words = self
                                .get_words_from_document(&tdp)
                                .into_iter()
                                .map(|w| w.to_lowercase())
                                .filter(|w| self.vcards.search_emails(w))
                                .collect::<Vec<_>>();
                            let response = if let Some(text) = self.vcards.hover(&words) {
                                let resp = lsp_types::Hover {
                                    contents: lsp_types::HoverContents::Markup(
                                        lsp_types::MarkupContent {
                                            kind: lsp_types::MarkupKind::Markdown,
                                            value: text,
                                        },
                                    ),
                                    range: None,
                                };
                                Message::Response(Response {
                                    id: r.id,
                                    result: Some(serde_json::to_value(resp).unwrap()),
                                    error: None,
                                })
                            } else {
                                Message::Response(Response {
                                    id: r.id,
                                    result: None,
                                    error: None,
                                })
                            };

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::GotoDefinition::METHOD => {
                            let tdp =
                                serde_json::from_value::<lsp_types::TextDocumentPositionParams>(
                                    r.params,
                                )
                                .unwrap();

                            let words = self.get_words_from_document(&tdp);
                            let words: Vec<_> =
                                words.into_iter().map(|w| w.to_lowercase()).collect();
                            let response = match self.vcards.all_info_file(&words) {
                                Some(filename) => {
                                    let resp =
                                        lsp_types::GotoDefinitionResponse::Scalar(Location {
                                            uri: Url::from_file_path(filename).unwrap(),
                                            range: Range::default(),
                                        });
                                    Message::Response(Response {
                                        id: r.id,
                                        result: serde_json::to_value(resp).ok(),
                                        error: None,
                                    })
                                }
                                None => Message::Response(Response {
                                    id: r.id,
                                    result: None,
                                    error: None,
                                }),
                            };

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::Completion::METHOD => {
                            let mut tdp = serde_json::from_value::<
                                lsp_types::TextDocumentPositionParams,
                            >(r.params)
                            .unwrap();

                            tdp.position.character -= 1;
                            let response = match self.get_words_from_document(&tdp).first() {
                                Some(word) => {
                                    let limit = 100;
                                    let lower_word = word.to_lowercase();
                                    let completion_items = self.vcards.complete(
                                        &lower_word,
                                        word.chars().next().map_or(false, |c| c.is_uppercase()),
                                        limit,
                                    );
                                    let resp =
                                        lsp_types::CompletionResponse::List(CompletionList {
                                            is_incomplete: completion_items.len() == limit,
                                            items: completion_items,
                                        });
                                    Message::Response(Response {
                                        id: r.id,
                                        result: serde_json::to_value(resp).ok(),
                                        error: None,
                                    })
                                }
                                None => Message::Response(Response {
                                    id: r.id,
                                    result: None,
                                    error: None,
                                }),
                            };

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::ResolveCompletionItem::METHOD => {
                            let mut ci =
                                serde_json::from_value::<lsp_types::CompletionItem>(r.params)
                                    .unwrap();

                            let lower_word = ci.label.to_lowercase();
                            let response = if let Some(doc) = self.vcards.hover(&[lower_word]) {
                                ci.documentation = Some(lsp_types::Documentation::MarkupContent(
                                    lsp_types::MarkupContent {
                                        kind: lsp_types::MarkupKind::Markdown,
                                        value: doc,
                                    },
                                ));
                                Message::Response(Response {
                                    id: r.id,
                                    result: serde_json::to_value(ci).ok(),
                                    error: None,
                                })
                            } else {
                                Message::Response(Response {
                                    id: r.id,
                                    result: None,
                                    error: None,
                                })
                            };

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::CodeActionRequest::METHOD => {
                            let cap =
                                serde_json::from_value::<lsp_types::CodeActionParams>(r.params)
                                    .unwrap();

                            let tdp = TextDocumentPositionParams {
                                text_document: cap.text_document,
                                position: cap.range.start,
                            };

                            let words = self.get_words_from_document(&tdp);
                            let completion_items = words
                                .into_iter()
                                .map(|w| w.to_lowercase())
                                .map(|w| {
                                    let args = serde_json::to_value(DefineCommandArguments {
                                        word: w.to_owned(),
                                    })
                                    .unwrap();
                                    lsp_types::CodeActionOrCommand::Command(lsp_types::Command {
                                        title: format!("Define {w:?}"),
                                        command: "define".to_owned(),
                                        arguments: Some(vec![args]),
                                    })
                                })
                                .collect::<Vec<_>>();
                            let response = Message::Response(Response {
                                id: r.id,
                                result: Some(serde_json::to_value(completion_items).unwrap()),
                                error: None,
                            });

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::ExecuteCommand::METHOD => {
                            let mut cap =
                                serde_json::from_value::<lsp_types::ExecuteCommandParams>(r.params)
                                    .unwrap();

                            let response = match cap.command.as_str() {
                                "define" => {
                                    let arg = cap.arguments.swap_remove(0);
                                    match serde_json::from_value::<DefineCommandArguments>(arg) {
                                        Ok(args) => match self.vcards.all_info_file(&[args.word]) {
                                            Some(filename) => {
                                                let params = ShowDocumentParams {
                                                    uri: Url::from_file_path(filename).unwrap(),
                                                    external: None,
                                                    take_focus: None,
                                                    selection: None,
                                                };
                                                c.sender
                                                    .send(Message::Request(Request {
                                                        id: RequestId::from(0),
                                                        method:
                                                            lsp_types::request::ShowDocument::METHOD
                                                                .to_owned(),
                                                        params: serde_json::to_value(params)
                                                            .unwrap(),
                                                    }))
                                                    .unwrap();
                                                Message::Response(Response {
                                                    id: r.id,
                                                    result: None,
                                                    error: None,
                                                })
                                            }
                                            None => Message::Response(Response {
                                                id: r.id,
                                                result: None,
                                                error: None,
                                            }),
                                        },
                                        _ => Message::Response(Response {
                                            id: r.id,
                                            result: None,
                                            error: Some(ResponseError {
                                                code: ErrorCode::InvalidRequest as i32,
                                                message: String::from("invalid arguments"),
                                                data: None,
                                            }),
                                        }),
                                    }
                                }
                                _ => Message::Response(Response {
                                    id: r.id,
                                    result: None,
                                    error: Some(ResponseError {
                                        code: ErrorCode::InvalidRequest as i32,
                                        message: String::from("unknown command"),
                                        data: None,
                                    }),
                                }),
                            };

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::Shutdown::METHOD => {
                            self.shutdown = true;
                            let none: Option<()> = None;
                            c.sender
                                .send(Message::Response(Response::new_ok(r.id, none)))
                                .unwrap()
                        }
                        _ => log(&c, format!("Unmatched request received: {}", r.method)),
                    }
                }
                Message::Response(r) => log(&c, format!("Unmatched response received: {}", r.id)),
                Message::Notification(n) => {
                    match &n.method[..] {
                        lsp_types::notification::DidOpenTextDocument::METHOD => {
                            let dotdp = serde_json::from_value::<
                                lsp_types::DidOpenTextDocumentParams,
                            >(n.params)
                            .unwrap();
                            self.open_files.insert(
                                dotdp.text_document.uri.to_string(),
                                dotdp.text_document.text,
                            );
                            // log(
                            //     &c,
                            //     format!(
                            //         "got open document notification for {:?}",
                            //         dotdp.text_document.uri
                            //     ),
                            // );
                        }
                        lsp_types::notification::DidChangeTextDocument::METHOD => {
                            let dctdp = serde_json::from_value::<
                                lsp_types::DidChangeTextDocumentParams,
                            >(n.params)
                            .unwrap();
                            let doc = dctdp.text_document.uri.to_string();
                            let content = self.open_files.get_mut(&doc).unwrap();
                            for change in dctdp.content_changes {
                                if let Some(range) = change.range {
                                    let start = resolve_position(content, range.start);
                                    let end = resolve_position(content, range.end);
                                    content.replace_range(start..end, &change.text);
                                } else {
                                    // full content replace
                                    *content = change.text;
                                }
                            }
                            // log(&c, format!("got change document notification for {doc:?}"))
                        }
                        lsp_types::notification::DidCloseTextDocument::METHOD => {
                            let dctdp = serde_json::from_value::<
                                lsp_types::DidCloseTextDocumentParams,
                            >(n.params)
                            .unwrap();
                            self.open_files.remove(&dctdp.text_document.uri.to_string());
                            // log(
                            //     &c,
                            //     format!(
                            //         "got close document notification for {:?}",
                            //         dctdp.text_document.uri
                            //     ),
                            // );
                        }
                        lsp_types::notification::Exit::METHOD => {
                            if self.shutdown {
                                return Ok(());
                            } else {
                                return Err(String::from(
                                    "Received exit notification before shutdown request",
                                ));
                            }
                        }
                        _ => log(&c, format!("Unmatched notification received: {}", n.method)),
                    }
                }
            }
        }
    }

    fn get_file_content(&self, uri: &Url) -> String {
        if let Some(content) = self.open_files.get(&uri.to_string()) {
            content.to_owned()
        } else {
            std::fs::read_to_string(uri.to_file_path().unwrap()).unwrap()
        }
    }

    fn get_words_from_document(&self, tdp: &lsp_types::TextDocumentPositionParams) -> Vec<String> {
        let content = self.get_file_content(&tdp.text_document.uri);
        get_words_from_content(
            &content,
            tdp.position.line as usize,
            tdp.position.character as usize,
        )
    }
}

fn get_words_from_content(content: &str, line: usize, character: usize) -> Vec<String> {
    let line = match content.lines().nth(line) {
        None => return Vec::new(),
        Some(l) => l,
    };

    let mut words = Vec::new();
    let mut current_word = String::new();
    if let Some(word) = get_word_from_line(line, character) {
        for single_word in word.split_whitespace() {
            if !current_word.is_empty() {
                current_word.push('_');
            }
            current_word.push_str(single_word);
            words.push(current_word.clone());
            // now try and simplify the word
            for c in WORD_PUNC.chars() {
                if let Some(w) = current_word.strip_prefix(c) {
                    words.push(w.to_owned());
                    if let Some(w) = w.strip_suffix(c) {
                        words.push(w.to_owned());
                    }
                }
                if let Some(w) = current_word.strip_suffix(c) {
                    words.push(w.to_owned());
                }
            }
        }
    }
    // sort by length to try and find the simplest
    words.sort_unstable_by(|s1, s2| {
        if s1.len() < s2.len() {
            Ordering::Less
        } else {
            s1.cmp(s2)
        }
    });
    words.dedup();
    words
}

const WORD_PUNC: &str = "_-'./";

fn get_word_from_line(line: &str, character: usize) -> Option<String> {
    let mut current_word = String::new();
    let mut found = false;
    let mut match_chars = WORD_PUNC.to_owned();
    let word_char = |match_with: &str, c: char| c.is_alphanumeric() || match_with.contains(c);
    for (i, c) in line.chars().enumerate() {
        if word_char(&match_chars, c) {
            current_word.push(c);
        } else {
            if found {
                return Some(current_word);
            }
            current_word.clear();
        }

        if i == character {
            if word_char(&match_chars, c) {
                match_chars.push(' ');
                found = true
            } else {
                return None;
            }
        }

        if !word_char(&match_chars, c) && found {
            return Some(current_word);
        }
    }

    // got to end of line
    if found {
        return Some(current_word);
    }

    None
}

fn main() {
    let args = Args::parse();
    let (p, c, io) = connect(args.stdio);
    let server = Server::new(&c, p);
    let s = server.serve(c);
    io.join().unwrap();
    match s {
        Ok(()) => (),
        Err(s) => {
            eprintln!("{}", s);
            std::process::exit(1)
        }
    }
}

struct VCards {
    root: PathBuf,
}

impl VCards {
    fn new(value: &Path) -> Self {
        Self {
            root: value.to_owned(),
        }
    }

    fn hover(&self, words: &[String]) -> Option<String> {
        todo!()
    }

    fn render_hover(&self, word: &str) -> String {
        todo!()
    }

    fn all_info_file(&self, words: &[String]) -> Option<PathBuf> {
        todo!()
    }

    fn all_info(&self, words: &[String]) -> Option<String> {
        todo!()
    }

    fn complete(&self, word: &String, capitalise: bool, limit: usize) -> Vec<CompletionItem> {
        todo!()
    }

    fn search_emails(&self, email: &String) -> bool{
        todo!()
    }
}

fn resolve_position(content: &str, pos: Position) -> usize {
    let count = content
        .lines()
        .map(|l| l.len())
        .take(pos.line as usize)
        .sum::<usize>();
    pos.line as usize + count + pos.character as usize
}

#[derive(Debug, Serialize, Deserialize)]
struct DefineCommandArguments {
    word: String,
}

#[cfg(test)]
mod tests {
}
