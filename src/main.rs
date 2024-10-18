use clap::Parser;
use itertools::Itertools;
use lsp_server::ErrorCode;
use lsp_server::Message;
use lsp_server::Notification;
use lsp_server::Response;
use lsp_server::ResponseError;
use lsp_server::{Connection, IoThreads};
use lsp_types::notification::LogMessage;
use lsp_types::notification::Notification as _;
use lsp_types::notification::ShowMessage;
use lsp_types::request::Request as _;
use lsp_types::CompletionItem;
use lsp_types::CompletionItemKind;
use lsp_types::CompletionList;
use lsp_types::InitializeParams;
use lsp_types::InitializeResult;
use lsp_types::Location;
use lsp_types::Position;
use lsp_types::PositionEncodingKind;
use lsp_types::Range;
use lsp_types::ServerCapabilities;
use lsp_types::ServerInfo;
use lsp_types::TextDocumentSyncKind;
use lsp_types::Url;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::read_dir;
use std::fs::read_to_string;
use std::path::PathBuf;
use vcard4::Vcard;

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
        // execute_command_provider: Some(ExecuteCommandOptions {
        //     commands: vec!["define".to_owned()],
        //     ..Default::default()
        // }),
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
            vcards: VCards::new(vcard_root),
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

                            let vcards = self
                                .get_word_from_document(&tdp)
                                .map(|w| w.to_lowercase())
                                .map(|w| self.vcards.find_by_email(&w))
                                .unwrap_or_default();
                            let response = if let Some(text) = self.vcards.hover(&vcards) {
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

                            let vcard_paths = self
                                .get_word_from_document(&tdp)
                                .map(|w| w.to_lowercase())
                                .map(|w| self.vcards.find_contact_paths_by_email(&w))
                                .unwrap_or_default();
                            let response = match vcard_paths.len() {
                                0 => Message::Response(Response {
                                    id: r.id,
                                    result: None,
                                    error: None,
                                }),
                                1 => {
                                    let resp =
                                        lsp_types::GotoDefinitionResponse::Scalar(Location {
                                            uri: Url::from_file_path(vcard_paths[0]).unwrap(),
                                            range: Range::default(),
                                        });
                                    Message::Response(Response {
                                        id: r.id,
                                        result: serde_json::to_value(resp).ok(),
                                        error: None,
                                    })
                                }
                                _ => {
                                    let resp = lsp_types::GotoDefinitionResponse::Array(
                                        vcard_paths
                                            .iter()
                                            .map(|p| Location {
                                                uri: Url::from_file_path(p).unwrap(),
                                                range: Range::default(),
                                            })
                                            .collect(),
                                    );
                                    Message::Response(Response {
                                        id: r.id,
                                        result: serde_json::to_value(resp).ok(),
                                        error: None,
                                    })
                                }
                            };

                            c.sender.send(response).unwrap()
                        }
                        lsp_types::request::Completion::METHOD => {
                            let mut tdp = serde_json::from_value::<
                                lsp_types::TextDocumentPositionParams,
                            >(r.params)
                            .unwrap();

                            tdp.position.character = tdp.position.character.saturating_sub(1);
                            let response = match self.get_word_from_document(&tdp) {
                                Some(word) => {
                                    let limit = 100;
                                    let lower_word = word.to_lowercase();
                                    let completion_items = self.vcards.complete(&lower_word, limit);
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
                            let vcards = self.vcards.find_by_email(&lower_word);
                            let response = if let Some(doc) = self.vcards.hover(&vcards) {
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
                            todo!()
                            // let cap =
                            //     serde_json::from_value::<lsp_types::CodeActionParams>(r.params)
                            //         .unwrap();
                            //
                            // let tdp = TextDocumentPositionParams {
                            //     text_document: cap.text_document,
                            //     position: cap.range.start,
                            // };
                            //
                            // let words = self.get_word_from_document(&tdp);
                            // let completion_items = words
                            //     .into_iter()
                            //     .map(|w| w.to_lowercase())
                            //     .map(|w| {
                            //         let args = serde_json::to_value(DefineCommandArguments {
                            //             word: w.to_owned(),
                            //         })
                            //         .unwrap();
                            //         lsp_types::CodeActionOrCommand::Command(lsp_types::Command {
                            //             title: format!("Define {w:?}"),
                            //             command: "define".to_owned(),
                            //             arguments: Some(vec![args]),
                            //         })
                            //     })
                            //     .collect::<Vec<_>>();
                            // let response = Message::Response(Response {
                            //     id: r.id,
                            //     result: Some(serde_json::to_value(completion_items).unwrap()),
                            //     error: None,
                            // });
                            //
                            // c.sender.send(response).unwrap()
                        }
                        lsp_types::request::ExecuteCommand::METHOD => {
                            todo!()
                            // let mut cap =
                            //     serde_json::from_value::<lsp_types::ExecuteCommandParams>(r.params)
                            //         .unwrap();
                            //
                            // let response = match cap.command.as_str() {
                            //     "define" => {
                            //         let arg = cap.arguments.swap_remove(0);
                            //         match serde_json::from_value::<DefineCommandArguments>(arg) {
                            //             Ok(args) => match self.vcards.all_info_file(&[args.word]) {
                            //                 Some(filename) => {
                            //                     let params = ShowDocumentParams {
                            //                         uri: Url::from_file_path(filename).unwrap(),
                            //                         external: None,
                            //                         take_focus: None,
                            //                         selection: None,
                            //                     };
                            //                     c.sender
                            //                         .send(Message::Request(Request {
                            //                             id: RequestId::from(0),
                            //                             method:
                            //                                 lsp_types::request::ShowDocument::METHOD
                            //                                     .to_owned(),
                            //                             params: serde_json::to_value(params)
                            //                                 .unwrap(),
                            //                         }))
                            //                         .unwrap();
                            //                     Message::Response(Response {
                            //                         id: r.id,
                            //                         result: None,
                            //                         error: None,
                            //                     })
                            //                 }
                            //                 None => Message::Response(Response {
                            //                     id: r.id,
                            //                     result: None,
                            //                     error: None,
                            //                 }),
                            //             },
                            //             _ => Message::Response(Response {
                            //                 id: r.id,
                            //                 result: None,
                            //                 error: Some(ResponseError {
                            //                     code: ErrorCode::InvalidRequest as i32,
                            //                     message: String::from("invalid arguments"),
                            //                     data: None,
                            //                 }),
                            //             }),
                            //         }
                            //     }
                            //     _ => Message::Response(Response {
                            //         id: r.id,
                            //         result: None,
                            //         error: Some(ResponseError {
                            //             code: ErrorCode::InvalidRequest as i32,
                            //             message: String::from("unknown command"),
                            //             data: None,
                            //         }),
                            //     }),
                            // };
                            //
                            // c.sender.send(response).unwrap()
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

    fn get_word_from_document(
        &self,
        tdp: &lsp_types::TextDocumentPositionParams,
    ) -> Option<String> {
        let content = self.get_file_content(&tdp.text_document.uri);
        get_word_from_content(
            &content,
            tdp.position.line as usize,
            tdp.position.character as usize,
        )
    }
}

fn get_word_from_content(content: &str, line: usize, character: usize) -> Option<String> {
    let line = content.lines().nth(line)?;
    let word = get_word_from_line(line, character)?;
    Some(word)
}

const EMAIL_PUNC: &str = "._%+-@";

fn get_word_from_line(line: &str, character: usize) -> Option<String> {
    let mut current_word = String::new();
    let mut found = false;
    let mut match_chars = EMAIL_PUNC.to_owned();
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
    vcards: BTreeMap<PathBuf, Vec<vcard4::Vcard>>,
}

impl VCards {
    fn new(value: PathBuf) -> Self {
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

    fn hover(&self, cards: &[&Vcard]) -> Option<String> {
        if cards.is_empty() {
            return None;
        }
        Some(
            cards
                .iter()
                .map(|vc| render_vcard(vc))
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }

    fn complete(&self, word: &str, limit: usize) -> Vec<CompletionItem> {
        self.vcards
            .values()
            .flatten()
            .filter(|vc| match_vcard(vc, word))
            .flat_map(|vc| vc.email.iter().map(|e| &e.value))
            .unique()
            .map(|email| CompletionItem {
                label: email.to_owned(),
                kind: Some(CompletionItemKind::TEXT),
                ..Default::default()
            })
            .take(limit)
            .collect()
    }

    fn find_by_email(&self, email: &str) -> Vec<&Vcard> {
        self.vcards
            .values()
            .flatten()
            .filter(|vc| vc.email.iter().any(|e| e.value.to_lowercase() == email))
            .collect()
    }

    fn find_contact_paths_by_email(&self, email: &str) -> Vec<&PathBuf> {
        self.vcards
            .iter()
            .filter(|(_, vcs)| {
                vcs.iter()
                    .any(|vc| vc.email.iter().any(|e| e.value.to_lowercase() == email))
            })
            .map(|(p, _)| p)
            .collect()
    }
}

fn render_vcard(vcard: &Vcard) -> String {
    let mut lines = Vec::new();
    if let Some(formatted_name) = vcard.formatted_name.first() {
        lines.push(format!("# {}", formatted_name.value));
    }
    if let Some(nick) = vcard.nickname.first() {
        lines.push(format!("_{}_", nick.value));
    }
    if !vcard.email.is_empty() {
        lines.push("Email addresses:".to_owned())
    }
    for e in vcard.email.iter().map(|e| format!("- {}", e.value)) {
        lines.push(e)
    }
    if !vcard.tel.is_empty() {
        lines.push("Telephone numbers:".to_owned())
    }
    for e in vcard.tel.iter().map(|e| format!("- {}", e)) {
        lines.push(e)
    }
    lines.join("\n")
}

fn resolve_position(content: &str, pos: Position) -> usize {
    let mut count = 0;
    let mut lines = 0;
    let mut character = 0;
    for c in content.chars() {
        count += 1;
        character += 1;
        if c == '\n' {
            lines += 1;
            character = 0;
        }
        if lines >= pos.line && character >= pos.character {
            break;
        }
    }
    count
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
