use crate::{
    completion_at, definition_range_at, hover_at, references_at,
    workspace_symbols_for_document, DocumentSnapshot,
};
use lsp_types::{
    CompletionOptions, CompletionParams, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    GotoDefinitionParams, GotoDefinitionResponse, HoverParams, InitializeResult, Location,
    OneOf, PublishDiagnosticsParams, ReferenceParams, RenameOptions, RenameParams,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    Response { id: Value, result: Value },
    Error { id: Option<Value>, code: i64, message: String },
    Notification { method: String, params: Value },
}

#[derive(Debug, Default)]
pub struct LanguageServerState {
    documents: HashMap<Url, String>,
    published: BTreeSet<Url>,
    shutdown_requested: bool,
    workspace_roots: Vec<std::path::PathBuf>,
}

impl LanguageServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&mut self, request: Value) -> Vec<ServerMessage> {
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = request.get("id").cloned();
        if self.shutdown_requested {
            return match id {
                Some(id) => vec![ServerMessage::Error {
                    id: Some(id),
                    code: -32600,
                    message: "language server has shut down".into(),
                }],
                None => Vec::new(),
            };
        }
        match method {
            "initialize" => {
                let params = &request["params"];
                self.workspace_roots = params["workspaceFolders"].as_array().into_iter().flatten()
                    .map(|folder| &folder["uri"])
                    .chain(std::iter::once(&params["rootUri"]))
                    .filter_map(|u| Url::parse(u.as_str()?).ok()?.to_file_path().ok())
                    .filter(|path| path.is_dir()).collect();
                self.workspace_roots.sort();
                self.workspace_roots.dedup();
                self.initialize(id)
            }
            "initialized" => Vec::new(),
            "shutdown" => {
                let Some(id) = id else { return Vec::new() };
                self.shutdown_requested = true;
                vec![ServerMessage::Response {
                    id,
                    result: Value::Null,
                }]
            }
            "exit" => Vec::new(),
            "textDocument/didOpen" => self.did_open(request.get("params").cloned()),
            "textDocument/didClose" => {
                let Ok(params) = parse_params::<lsp_types::DidCloseTextDocumentParams>(request.get("params").cloned()) else { return Vec::new() };
                let uri = params.text_document.uri;
                self.documents.remove(&uri);
                self.publish_diagnostics()
            }
            "textDocument/didChange" => self.did_change(request.get("params").cloned()),
            "textDocument/didSave" => self.did_save(request.get("params").cloned()),
            "textDocument/hover" => self.hover(id, request.get("params").cloned()),
            "textDocument/completion" => self.completion(id, request.get("params").cloned()),
            "textDocument/definition" => self.definition(id, request.get("params").cloned()),
            "textDocument/references" => self.references(id, request.get("params").cloned()),
            "textDocument/rename" => self.rename(id, request.get("params").cloned()),
            "workspace/symbol" => self.workspace_symbol(id, request.get("params").cloned()),
            _ if id.is_some() => vec![ServerMessage::Error {
                id,
                code: -32601,
                message: format!("method `{method}` is not supported by corvid-lsp yet"),
            }],
            _ => Vec::new(),
        }
    }

    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    fn initialize(&self, id: Option<Value>) -> Vec<ServerMessage> {
        let result = InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        " ".to_string(),
                        ".".to_string(),
                        ":".to_string(),
                        ">".to_string(),
                    ]),
                    ..CompletionOptions::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(false),
                    work_done_progress_options: Default::default(),
                })),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
            server_info: Some(lsp_types::ServerInfo {
                name: "corvid-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        };
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result: serde_json::to_value(result).expect("initialize result serializes"),
        }]
    }

    fn did_open(&mut self, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<DidOpenTextDocumentParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let uri = params.text_document.uri;
        self.documents
            .insert(uri.clone(), params.text_document.text);
        self.publish_diagnostics()
    }

    fn did_change(&mut self, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<DidChangeTextDocumentParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let Some(change) = params.content_changes.into_iter().last() else {
            return Vec::new();
        };
        let uri = params.text_document.uri;
        self.documents.insert(uri.clone(), change.text);
        self.publish_diagnostics()
    }

    fn did_save(&mut self, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match params {
            Some(params) => params,
            None => return Vec::new(),
        };
        let Some(_) = params
            .get("textDocument")
            .and_then(|doc| doc.get("uri"))
            .and_then(Value::as_str)
            .and_then(|uri| Url::parse(uri).ok())
        else {
            return Vec::new();
        };
        self.publish_diagnostics()
    }

    fn hover(&self, id: Option<Value>, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<HoverParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let result = self
            .documents
            .get(&uri)
            .and_then(|text| crate::navigation::imported_symbol_at(&self.documents, &uri, text, position)
                .and_then(|symbol| symbol.hover).or_else(|| hover_at(text, position)))
            .map(serde_json::to_value)
            .transpose()
            .expect("hover serializes")
            .unwrap_or(Value::Null);
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result,
        }]
    }

    fn completion(&self, id: Option<Value>, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<CompletionParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let result = self
            .documents
            .get(&uri)
            .map(|text| crate::completion::imported_members_at(&self.documents, &uri, text, position)
                .unwrap_or_else(|| completion_at(text, position)))
            .map(serde_json::to_value)
            .transpose()
            .expect("completion serializes")
            .unwrap_or(Value::Null);
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result,
        }]
    }

    fn definition(&self, id: Option<Value>, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<GotoDefinitionParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let result = self
            .documents
            .get(&uri)
            .and_then(|text| crate::navigation::imported_symbol_at(&self.documents, &uri, text, position)
                .map(|symbol| symbol.location)
                .or_else(|| definition_range_at(text, position).map(|range| Location { uri, range })))
            .map(GotoDefinitionResponse::Scalar)
            .map(serde_json::to_value)
            .transpose()
            .expect("definition serializes")
            .unwrap_or(Value::Null);
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result,
        }]
    }

    fn references(&self, id: Option<Value>, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<ReferenceParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let result = self
            .documents
            .get(&uri)
            .map(|text| {
                references_at(text, position)
                    .into_iter()
                    .map(|range| Location {
                        uri: uri.clone(),
                        range,
                    })
                    .collect::<Vec<_>>()
            })
            .map(serde_json::to_value)
            .transpose()
            .expect("references serialize")
            .unwrap_or(Value::Null);
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result,
        }]
    }

    fn rename(&self, id: Option<Value>, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<RenameParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let result = crate::navigation::workspace_rename(
            &self.documents, &uri, position, &params.new_name, &self.workspace_roots,
        )
            .map(serde_json::to_value)
            .transpose()
            .expect("rename edit serializes")
            .unwrap_or(Value::Null);
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result,
        }]
    }

    fn workspace_symbol(&self, id: Option<Value>, params: Option<Value>) -> Vec<ServerMessage> {
        let params = match parse_params::<WorkspaceSymbolParams>(params) {
            Ok(params) => params,
            Err(error) => return vec![error],
        };
        let symbols = self
            .documents
            .iter()
            .flat_map(|(uri, text)| {
                workspace_symbols_for_document(uri.clone(), text, params.query.as_str())
            })
            .collect::<Vec<_>>();
        let result = serde_json::to_value(WorkspaceSymbolResponse::Flat(symbols))
            .expect("workspace symbols serialize");
        vec![ServerMessage::Response {
            id: id.unwrap_or(Value::Null),
            result,
        }]
    }

    fn publish_diagnostics(&mut self) -> Vec<ServerMessage> {
        let mut by_uri = BTreeMap::<Url, Vec<lsp_types::Diagnostic>>::new();
        let buffers: HashMap<_, _> = self.documents.iter().filter_map(|(uri, text)| {
            let path = uri.to_file_path().ok()?;
            Some((path.canonicalize().unwrap_or(path), text))
        }).collect();
        let open_source = |path: &std::path::Path| {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
            buffers.get(&canonical).map(|text| text.as_bytes().to_vec())
        };
        for (uri, text) in &self.documents {
            let result = crate::analysis::analyze_buffers(&DocumentSnapshot { uri: uri.clone(), text: text.clone() }, &open_source);
            by_uri.insert(uri.clone(), result.diagnostics);
            for (origin, diagnostics) in result.imported {
                // The open buffer supplies diagnostics for its own document.
                if self.documents.contains_key(&origin) {
                    continue;
                }
                let entries = by_uri.entry(origin).or_default();
                for diagnostic in diagnostics {
                    if !entries.contains(&diagnostic) {
                        entries.push(diagnostic);
                    }
                }
            }
        }
        let current = by_uri.keys().cloned().collect();
        for uri in self.published.difference(&current) {
            by_uri.insert(uri.clone(), Vec::new());
        }
        self.published = current;
        by_uri.into_iter().map(|(uri, diagnostics)| ServerMessage::Notification {
            method: "textDocument/publishDiagnostics".into(),
            params: serde_json::to_value(PublishDiagnosticsParams { uri, diagnostics, version: None })
                .expect("diagnostics params serialize"),
        }).collect()
    }
}

impl ServerMessage {
    pub fn into_json(self) -> Value {
        match self {
            ServerMessage::Response { id, result } => {
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            ServerMessage::Error { id, code, message } => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id.unwrap_or(Value::Null),
                    "error": { "code": code, "message": message }
                })
            }
            ServerMessage::Notification { method, params } => {
                json!({ "jsonrpc": "2.0", "method": method, "params": params })
            }
        }
    }
}

fn parse_params<T>(params: Option<Value>) -> Result<T, ServerMessage>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(params.unwrap_or(Value::Null)).map_err(|error| ServerMessage::Error {
        id: None,
        code: -32602,
        message: format!("invalid LSP params: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn did_open(uri: &str, text: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": "corvid",
                    "version": 1,
                    "text": text
                }
            }
        })
    }

    #[test]
    fn initialize_advertises_full_text_sync() {
        let mut server = LanguageServerState::new();
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }));
        assert_eq!(messages.len(), 1);
        let json = messages.into_iter().next().unwrap().into_json();
        assert_eq!(json["id"], 1);
        assert_eq!(
            json["result"]["capabilities"]["textDocumentSync"].as_i64(),
            Some(1)
        );
        assert_eq!(json["result"]["capabilities"]["hoverProvider"], true);
        assert!(json["result"]["capabilities"]["completionProvider"].is_object());
        assert_eq!(json["result"]["capabilities"]["definitionProvider"], true);
        assert_eq!(json["result"]["capabilities"]["referencesProvider"], true);
        assert_eq!(json["result"]["capabilities"]["workspaceSymbolProvider"], true);
    }

    #[test]
    fn unsaved_module_changes_refresh_the_callers_diagnostics() {
        let root = std::env::temp_dir().join("corvid-lsp-buffer-only-project");
        let main = Url::from_file_path(root.join("main.cor")).unwrap();
        let helper = Url::from_file_path(root.join("helper.cor")).unwrap();
        let mut server = LanguageServerState::new();
        let function = "public fn measure(x: Int) -> Int:\n    return 1\n";
        server.handle(did_open(helper.as_str(), function));
        let caller = "import \"./helper\" use measure\nagent main() -> Int:\n    return measure(4)\n";
        let reports = server.handle(did_open(main.as_str(), caller));
        assert!(reports.into_iter().all(|message| message.into_json()["params"]["diagnostics"] == json!([])));
        let reports = server.handle(json!({
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {"uri": helper, "version": 2},
                "contentChanges": [{"text": function.replace("x: Int", "x: String")}]
            }
        }));
        let at_main = reports.into_iter().map(ServerMessage::into_json)
            .find(|message| message["params"]["uri"] == json!(main)).unwrap();
        let error = &at_main["params"]["diagnostics"][0];
        assert_eq!(error["range"]["start"]["line"], 2);
        assert!(error["message"].as_str().unwrap().contains("String"));
    }

    #[test]
    fn did_open_publishes_compiler_diagnostics() {
        let mut server = LanguageServerState::new();
        let messages = server.handle(did_open(
            "file:///workspace/main.cor",
            "agent answer() -> Int:\n    return missing_name\n",
        ));
        assert_eq!(messages.len(), 1);
        let json = messages.into_iter().next().unwrap().into_json();
        assert_eq!(json["method"], "textDocument/publishDiagnostics");
        let diagnostics = json["params"]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("missing_name"));
    }

    #[test]
    fn did_change_replaces_document_and_clears_diagnostics() {
        let mut server = LanguageServerState::new();
        server.handle(did_open(
            "file:///workspace/main.cor",
            "agent answer() -> Int:\n    return missing_name\n",
        ));
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///workspace/main.cor", "version": 2 },
                "contentChanges": [{ "text": "agent answer() -> Int:\n    return 42\n" }]
            }
        }));
        let json = messages.into_iter().next().unwrap().into_json();
        assert_eq!(
            json["params"]["diagnostics"].as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn hover_returns_compiler_backed_signature() {
        let mut server = LanguageServerState::new();
        server.handle(did_open(
            "file:///workspace/main.cor",
            "agent answer(x: Int) -> Int:\n    return x + 1\n",
        ));
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///workspace/main.cor" },
                "position": { "line": 0, "character": 7 }
            }
        }));
        let json = messages.into_iter().next().unwrap().into_json();
        assert_eq!(json["id"], 2);
        assert!(json["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("agent answer(x: Int) -> Int"));
    }

    #[test]
    fn completion_returns_contextual_approval_label() {
        let mut server = LanguageServerState::new();
        server.handle(did_open(
            "file:///workspace/main.cor",
            "tool issue_refund(id: String) -> String dangerous\n\nagent run(id: String) -> String:\n    approve \n",
        ));
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": "file:///workspace/main.cor" },
                "position": { "line": 3, "character": 12 }
            }
        }));
        let json = messages.into_iter().next().unwrap().into_json();
        assert_eq!(json["id"], 3);
        let labels = json["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["IssueRefund".to_string()]);
    }

    #[test]
    fn definition_returns_tool_location() {
        let mut server = LanguageServerState::new();
        server.handle(did_open(
            "file:///workspace/main.cor",
            "tool get_order(id: String) -> String\n\nagent run(id: String) -> String:\n    return get_order(id)\n",
        ));
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": "file:///workspace/main.cor" },
                "position": { "line": 3, "character": 13 }
            }
        }));
        let json = messages.into_iter().next().unwrap().into_json();
        assert_eq!(json["id"], 4);
        assert_eq!(json["result"]["range"]["start"]["line"], 0);
    }

    #[test]
    fn rename_returns_workspace_edit_for_local_identity() {
        let mut server = LanguageServerState::new();
        server.handle(did_open(
            "file:///workspace/main.cor",
            "agent run(id: String) -> String:\n    return id\n",
        ));
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": "file:///workspace/main.cor" },
                "position": { "line": 0, "character": 11 },
                "newName": "ticket_id"
            }
        }));
        let json = messages.into_iter().next().unwrap().into_json();
        let edits = json["result"]["changes"]["file:///workspace/main.cor"]
            .as_array()
            .unwrap();
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn workspace_symbol_returns_open_document_symbols() {
        let mut server = LanguageServerState::new();
        server.handle(did_open(
            "file:///workspace/main.cor",
            "effect retrieval:\n    data: grounded\n\nagent search() -> String:\n    return \"ok\"\n",
        ));
        let messages = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "workspace/symbol",
            "params": { "query": "retr" }
        }));
        let json = messages.into_iter().next().unwrap().into_json();
        let symbols = json["result"].as_array().unwrap();
        assert_eq!(symbols[0]["name"], "retrieval");
    }
}
