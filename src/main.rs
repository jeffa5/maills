use clap::Parser;
use line_index::LineIndex;
use line_index::TextSize;
use lsp_server::ErrorCode;
use lsp_server::Message;
use lsp_server::Notification;
use lsp_server::Request;
use lsp_server::RequestId;
use lsp_server::Response;
use lsp_server::{Connection, IoThreads};
use lsp_types::notification::LogMessage;
use lsp_types::notification::Notification as _;
use lsp_types::notification::PublishDiagnostics;
use lsp_types::notification::ShowMessage;
use lsp_types::request::Request as _;
use lsp_types::CodeActionKind;
use lsp_types::CompletionItem;
use lsp_types::CompletionItemKind;
use lsp_types::CompletionList;
use lsp_types::Diagnostic;
use lsp_types::DiagnosticSeverity;
use lsp_types::ExecuteCommandOptions;
use lsp_types::InitializeParams;
use lsp_types::InitializeResult;
use lsp_types::Position;
use lsp_types::PositionEncodingKind;
use lsp_types::PublishDiagnosticsParams;
use lsp_types::Range;
use lsp_types::ServerCapabilities;
use lsp_types::ServerInfo;
use lsp_types::ShowDocumentParams;
use lsp_types::TextDocumentPositionParams;
use lsp_types::TextDocumentSyncKind;
use lsp_types::Url;
use maills::ContactList;
use maills::ContactSource as _;
use maills::Mailbox;
use maills::OpenFiles;
use maills::Sources;
use maills::VCards;
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use std::str::FromStr;

const CREATE_CONTACT_COMMAND: &str = "create_contact";

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

fn notify(c: &Connection, method: &str, params: impl Serialize) {
    c.sender
        .send(Message::Notification(Notification::new(
            method.to_owned(),
            params,
        )))
        .unwrap();
}

fn response_empty(id: RequestId) -> Message {
    Message::Response(Response {
        id,
        result: None,
        error: None,
    })
}

fn response_ok(id: RequestId, result: impl Serialize) -> Message {
    Message::Response(Response::new_ok(id, result))
}

fn response_err(id: RequestId, code: i32, message: String) -> Message {
    Message::Response(Response::new_err(id, code, message))
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
            commands: vec![CREATE_CONTACT_COMMAND.to_owned()],
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
                notify(
                    &connection,
                    ShowMessage::METHOD,
                    format!("Invalid initialization options: {err}"),
                );
                panic!("Invalid initialization options: {err}")
            }
        }
    } else {
        notify(
            &connection,
            ShowMessage::METHOD,
            "No initialization options given, need it for vcard directory location at least",
        );
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
    sources: Sources,
    open_files: OpenFiles,
    diagnostics: Vec<Diagnostic>,
    shutdown: bool,
}

#[derive(Serialize, Deserialize)]
struct InitializationOptions {
    vcard_dir: Option<PathBuf>,
    contact_list_file: Option<PathBuf>,
    contact_list_diagnostics: Option<bool>,
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
                    notify(
                        c,
                        ShowMessage::METHOD,
                        format!("Invalid initialization options: {err}"),
                    );
                    panic!("Invalid initialization options: {err}")
                }
            }
        } else {
            notify(
                c,
                ShowMessage::METHOD,
                "No initialization options given, need it for vcard directory location at least",
            );
            panic!("No initialization options given, need it for vcard directory location at least")
        };
        let mut sources = Sources::default();
        if let Some(vcard_dir) = init_opts.vcard_dir {
            let vcard_root = if vcard_dir.starts_with("~/") {
                dirs::home_dir()
                    .unwrap()
                    .join(vcard_dir.strip_prefix("~/").unwrap())
            } else {
                vcard_dir
            };
            sources.sources.push(Box::new(VCards::new(vcard_root)));
        }

        if let Some(contact_list_file) = init_opts.contact_list_file {
            let contact_list_file = if contact_list_file.starts_with("~/") {
                dirs::home_dir()
                    .unwrap()
                    .join(contact_list_file.strip_prefix("~/").unwrap())
            } else {
                contact_list_file
            };
            let contact_list_diagnostics = init_opts.contact_list_diagnostics.unwrap_or(false);
            sources.sources.push(Box::new(ContactList::new(
                contact_list_file,
                contact_list_diagnostics,
            )));
        }

        if sources.sources.is_empty() {
            panic!("Initialization options must specify at least one of `vcard_dir` or `contact_list_file`");
        }

        Self {
            sources,
            open_files: OpenFiles::default(),
            diagnostics: Vec::new(),
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
                            .send(response_err(
                                r.id,
                                ErrorCode::InvalidRequest as i32,
                                String::from("received request after shutdown"),
                            ))
                            .unwrap();
                        continue;
                    }

                    let messages = match &r.method[..] {
                        lsp_types::request::HoverRequest::METHOD => self.handle_hover_request(r),
                        lsp_types::request::GotoDefinition::METHOD => {
                            self.handle_goto_definition_request(r)
                        }
                        lsp_types::request::Completion::METHOD => self.handle_completion_request(r),
                        lsp_types::request::ResolveCompletionItem::METHOD => {
                            self.handle_resolve_completion_item_request(r)
                        }
                        lsp_types::request::CodeActionRequest::METHOD => {
                            self.handle_code_action_request(r)
                        }
                        lsp_types::request::ExecuteCommand::METHOD => {
                            self.handle_execute_command_request(r)
                        }
                        lsp_types::request::Shutdown::METHOD => {
                            self.shutdown = true;
                            vec![response_empty(r.id)]
                        }
                        _ => {
                            log(&c, format!("Unmatched request received: {}", r.method));
                            vec![]
                        }
                    };
                    for message in messages {
                        c.sender.send(message).unwrap();
                    }
                }
                Message::Response(r) => log(&c, format!("Unmatched response received: {}", r.id)),
                Message::Notification(n) => {
                    let messages = match &n.method[..] {
                        lsp_types::notification::DidOpenTextDocument::METHOD => {
                            self.handle_did_open_text_document_notification(n)
                        }
                        lsp_types::notification::DidChangeTextDocument::METHOD => {
                            self.handle_did_change_text_document_notification(n)
                        }
                        lsp_types::notification::DidCloseTextDocument::METHOD => {
                            self.handle_did_close_text_document_notification(n)
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
                        _ => {
                            log(&c, format!("Unmatched notification received: {}", n.method));
                            Vec::new()
                        }
                    };
                    for message in messages {
                        c.sender.send(message).unwrap()
                    }
                }
            }
        }
    }

    fn handle_hover_request(&mut self, request: Request) -> Vec<Message> {
        let tdp = serde_json::from_value::<lsp_types::TextDocumentPositionParams>(request.params)
            .unwrap();

        let mailbox = self.get_mailbox_from_document(&tdp);
        let response = if let Some(mailbox) = mailbox {
            let text = self.sources.render(&mailbox);
            let resp = lsp_types::Hover {
                contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: text,
                }),
                range: None,
            };
            response_ok(request.id, resp)
        } else {
            response_empty(request.id)
        };

        vec![response]
    }

    fn handle_goto_definition_request(&mut self, request: Request) -> Vec<Message> {
        let tdp = serde_json::from_value::<lsp_types::TextDocumentPositionParams>(request.params)
            .unwrap();

        let mut locations = self
            .get_mailbox_from_document(&tdp)
            .map(|mailbox| self.sources.locations(&mailbox))
            .unwrap_or_default();
        let response = match locations.len() {
            0 => response_empty(request.id),
            1 => {
                let resp = lsp_types::GotoDefinitionResponse::Scalar(locations.remove(0).into());
                response_ok(request.id, resp)
            }
            _ => {
                let resp = lsp_types::GotoDefinitionResponse::Array(
                    locations.into_iter().map(|p| p.into()).collect(),
                );
                response_ok(request.id, resp)
            }
        };

        vec![response]
    }

    fn handle_completion_request(&mut self, request: Request) -> Vec<Message> {
        let mut tdp =
            serde_json::from_value::<lsp_types::TextDocumentPositionParams>(request.params)
                .unwrap();

        tdp.position.character = tdp.position.character.saturating_sub(1);
        let response = match self.get_word_from_document(&tdp) {
            Some(word) => {
                let limit = 100;
                let lower_word = word.to_lowercase();
                let matches = self.sources.find_matching(lower_word);
                let completion_items = matches
                    .map(|(source, mailbox)| CompletionItem {
                        label: mailbox.to_string(),
                        kind: Some(CompletionItemKind::TEXT),
                        label_details: Some(lsp_types::CompletionItemLabelDetails {
                            detail: Some(source.to_owned()),
                            description: None,
                        }),
                        ..Default::default()
                    })
                    .take(limit)
                    .collect::<Vec<_>>();
                let resp = lsp_types::CompletionResponse::List(CompletionList {
                    is_incomplete: completion_items.len() == limit,
                    items: completion_items,
                });
                response_ok(request.id, resp)
            }
            None => response_empty(request.id),
        };

        vec![response]
    }

    fn handle_resolve_completion_item_request(&mut self, request: Request) -> Vec<Message> {
        let mut ci = serde_json::from_value::<lsp_types::CompletionItem>(request.params).unwrap();

        let mailbox = Mailbox::from_str(&ci.label).unwrap();
        let doc = self.sources.render(&mailbox);
        ci.documentation = Some(lsp_types::Documentation::MarkupContent(
            lsp_types::MarkupContent {
                kind: lsp_types::MarkupKind::Markdown,
                value: doc,
            },
        ));
        let response = response_ok(request.id, ci);

        vec![response]
    }

    fn handle_code_action_request(&mut self, request: Request) -> Vec<Message> {
        let cap = serde_json::from_value::<lsp_types::CodeActionParams>(request.params).unwrap();

        let tdp = TextDocumentPositionParams {
            text_document: cap.text_document,
            position: cap.range.start,
        };

        let mut action_list = Vec::new();
        if let Some(mailbox) = self.get_mailbox_from_document(&tdp) {
            let args = serde_json::to_value(CreateContactCommandArguments { mailbox }).unwrap();
            let fixed_diagnostics = self
                .diagnostics
                .iter()
                .filter(|d| in_range(&d.range, &cap.range.start))
                .cloned()
                .collect::<Vec<_>>();
            let action = lsp_types::CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
                title: "Add to contacts".to_owned(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: if fixed_diagnostics.is_empty() {
                    None
                } else {
                    Some(fixed_diagnostics)
                },
                command: Some(lsp_types::Command {
                    title: "Add to contacts".to_owned(),
                    command: CREATE_CONTACT_COMMAND.to_owned(),
                    arguments: Some(vec![args]),
                }),
                ..Default::default()
            });
            action_list.push(action);
        }
        let response = response_ok(request.id, action_list);

        vec![response]
    }

    fn handle_execute_command_request(&mut self, request: Request) -> Vec<Message> {
        let mut cap =
            serde_json::from_value::<lsp_types::ExecuteCommandParams>(request.params).unwrap();

        let mut messages = Vec::new();
        let response = match cap.command.as_str() {
            CREATE_CONTACT_COMMAND => {
                let arg = cap.arguments.swap_remove(0);
                match serde_json::from_value::<CreateContactCommandArguments>(arg) {
                    Ok(args) => {
                        let path = self.sources.create_contact(args.mailbox);
                        if let Some(path) = path {
                            let params = ShowDocumentParams {
                                uri: Url::from_file_path(path).unwrap(),
                                external: None,
                                take_focus: None,
                                selection: None,
                            };
                            messages.push(Message::Request(lsp_server::Request {
                                id: RequestId::from(0),
                                method: lsp_types::request::ShowDocument::METHOD.to_owned(),
                                params: serde_json::to_value(params).unwrap(),
                            }));
                        }
                        response_empty(request.id)
                    }
                    _ => response_err(
                        request.id,
                        ErrorCode::InvalidRequest as i32,
                        String::from("invalid arguments"),
                    ),
                }
            }
            _ => response_err(
                request.id,
                ErrorCode::InvalidRequest as i32,
                String::from("unknown command"),
            ),
        };
        messages.push(response);

        messages
    }

    fn handle_did_open_text_document_notification(
        &mut self,
        notification: Notification,
    ) -> Vec<Message> {
        let dotdp =
            serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(notification.params)
                .unwrap();
        self.open_files.add(
            dotdp.text_document.uri.to_string(),
            dotdp.text_document.text,
        );
        let diagnostics = self.refresh_diagnostics(dotdp.text_document.uri.as_ref());
        let message = Message::Notification(Notification::new(
            PublishDiagnostics::METHOD.to_owned(),
            PublishDiagnosticsParams {
                uri: dotdp.text_document.uri,
                diagnostics,
                version: Some(dotdp.text_document.version),
            },
        ));
        vec![message]
        // log(
        //     &c,
        //     format!(
        //         "got open document notification for {:?}",
        //         dotdp.text_document.uri
        //     ),
        // );
    }

    fn handle_did_change_text_document_notification(
        &mut self,
        notification: Notification,
    ) -> Vec<Message> {
        let dctdp =
            serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(notification.params)
                .unwrap();
        let doc = dctdp.text_document.uri.to_string();
        self.open_files.apply_changes(&doc, dctdp.content_changes);
        let diagnostics = self.refresh_diagnostics(dctdp.text_document.uri.as_ref());
        let message = Message::Notification(Notification::new(
            PublishDiagnostics::METHOD.to_owned(),
            PublishDiagnosticsParams {
                uri: dctdp.text_document.uri,
                diagnostics,
                version: Some(dctdp.text_document.version),
            },
        ));
        vec![message]
        // log(&c, format!("got change document notification for {doc:?}"))
    }

    fn handle_did_close_text_document_notification(
        &mut self,
        notification: Notification,
    ) -> Vec<Message> {
        let dctdp =
            serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(notification.params)
                .unwrap();
        self.open_files.remove(dctdp.text_document.uri.as_ref());
        Vec::new()
        // log(
        //     &c,
        //     format!(
        //         "got close document notification for {:?}",
        //         dctdp.text_document.uri
        //     ),
        // );
    }

    fn get_mailbox_from_document(
        &mut self,
        tdp: &lsp_types::TextDocumentPositionParams,
    ) -> Option<Mailbox> {
        let content = self.open_files.get(tdp.text_document.uri.as_ref());
        get_mailbox_from_content(
            content,
            tdp.position.line as usize,
            tdp.position.character as usize,
        )
    }

    fn get_word_from_document(
        &mut self,
        tdp: &lsp_types::TextDocumentPositionParams,
    ) -> Option<String> {
        let content = self.open_files.get(tdp.text_document.uri.as_ref());
        get_word_from_content(
            content,
            tdp.position.line as usize,
            tdp.position.character as usize,
        )
    }

    fn refresh_diagnostics(&mut self, file: &str) -> Vec<Diagnostic> {
        let content = self.open_files.get(file);
        // from https://www.regular-expressions.info/email.html
        let re = regex::Regex::new(r"(?i)\b([A-Z0-9._%+-~/]+@[A-Z0-9.-]+\.[A-Z]{2,})\b").unwrap();
        let mut email_locations = Vec::new();
        for mtch in re.find_iter(content) {
            let start = mtch.start();
            let end = mtch.end();
            let email = mtch.as_str();
            email_locations.push((email, start, end));
        }
        let diagnostics = email_locations
            .iter()
            .filter(|(e, _, _)| !self.sources.contains(e))
            .map(|(_, start, end)| {
                let li = LineIndex::new(content);
                let start = li.line_col(TextSize::new(*start as u32));
                let end = li.line_col(TextSize::new(*end as u32));
                Diagnostic {
                    range: Range::new(
                        Position::new(start.line, start.col),
                        Position::new(end.line, end.col),
                    ),
                    severity: Some(DiagnosticSeverity::HINT),
                    // source: todo!(),
                    message: "Address is not in contacts".to_owned(),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        self.diagnostics = diagnostics.clone();
        diagnostics
    }
}

fn get_mailbox_from_content(content: &str, line: usize, character: usize) -> Option<Mailbox> {
    let line = content.lines().nth(line)?;
    Mailbox::from_line_at(line, character)
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

#[derive(Debug, Serialize, Deserialize)]
struct CreateContactCommandArguments {
    mailbox: Mailbox,
}

fn in_range(range: &Range, position: &Position) -> bool {
    (range.start.line < position.line
        || (range.start.line == position.line && range.start.character <= position.character))
        && (range.end.line > position.line
            || (range.end.line == position.line && range.end.character > position.character))
}
