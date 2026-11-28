use crate::{byte_span_to_lsp_range, lsp_position_to_byte};
use corvid_ast::{
    AgentDecl, Decl, EffectDecl, EvalDecl, File, FixtureDecl, ModelDecl, MockDecl, PromptDecl,
    Span, TestDecl, ToolDecl, TypeDecl,
};
use corvid_resolve::{resolve, Binding, DeclKind, DefId, LocalId, Resolved};
use corvid_syntax::{lex, parse_file};
use lsp_types::{Location, Position, Range, SymbolInformation, SymbolKind, Url};
use std::collections::BTreeSet;

mod rename;
pub(crate) use rename::workspace_rename;

pub(crate) struct ImportedSymbol {
    pub location: Location,
    pub hover: Option<lsp_types::Hover>,
}

pub(crate) fn imported_module_source<'a>(
    documents: &'a std::collections::HashMap<Url, String>,
    importer: &Url,
    import: &corvid_ast::ImportDecl,
) -> Option<(Url, std::borrow::Cow<'a, str>)> {
    use std::borrow::Cow;
    let importer_path = importer.to_file_path().ok()?;
    match import.source {
        corvid_ast::ImportSource::PackageCorvid => {
            let (path, text) = corvid_driver::installed_component_source(&importer_path, &import.module)?;
            return Some((Url::from_file_path(path).ok()?, Cow::Owned(text)));
        }
        corvid_ast::ImportSource::Corvid => {}
        _ => return None,
    }
    let path = corvid_resolve::resolve_import_path(&importer_path, &import.module);
    let uri = Url::from_file_path(&path).ok()?;
    if let Some(text) = documents.get(&uri) {
        return Some((uri, Cow::Borrowed(text)));
    }
    let canonical = path.canonicalize().ok()?;
    for (open, text) in documents {
        if open.to_file_path().ok().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&canonical) {
            return Some((open.clone(), Cow::Borrowed(text)));
        }
    }
    Some((uri, Cow::Owned(std::fs::read_to_string(path).ok()?)))
}

/// Follow a selective import or a module-qualified expression. The receiver
/// must bind to the import, since a parameter can shadow the module name.
pub(crate) fn imported_symbol_at(
    documents: &std::collections::HashMap<Url, String>,
    uri: &Url,
    source: &str,
    position: Position,
) -> Option<ImportedSymbol> {
    use corvid_syntax::TokKind;
    let offset = lsp_position_to_byte(source, position);
    let tokens = lex(source).ok()?;
    if !tokens.iter().any(|token| matches!(token.kind, TokKind::KwImport)) {
        return None;
    }
    let index = tokens
        .iter()
        .position(|t| matches!(t.kind, TokKind::Ident(_)) && contains(t.span, offset))?;
    let token = &tokens[index];
    let TokKind::Ident(name) = &token.kind else {
        return None;
    };
    let (file, _) = parse_file(&tokens);
    let resolved = resolve(&file);
    let bound_to = |span, kind, binding: &str| {
        matches!(resolved.bindings.get(&span),
        Some(Binding::Decl(id)) if resolved.symbols.get(*id).kind == kind
            && resolved.symbols.lookup_def(binding) == Some(*id))
    };
    for decl in &file.decls {
        let Decl::Import(import) = decl else { continue };
        let selective = import.use_items.iter().find(|item| {
            let local = item.alias.as_ref().unwrap_or(&item.name);
            token.span == item.name.span
                || token.span == local.span
                || (local.name == *name && bound_to(token.span, DeclKind::ImportedUse, name))
        });
        let member = if let Some(item) = selective {
            &item.name.name
        } else if index >= 2
            && matches!(tokens[index - 1].kind, TokKind::Dot)
            && import.alias.as_ref().is_some_and(|alias| {
                bound_to(tokens[index - 2].span, DeclKind::Import, &alias.name)
            })
        {
            name
        } else {
            continue;
        };
        let (target_uri, text) = imported_module_source(documents, uri, import)?;
        let target = text.as_ref();
        let (file, _) = parse_file(&lex(target).ok()?);
        let resolved = resolve(&file);
        let exports = corvid_resolve::collect_public_exports(&file, &resolved);
        let export = exports.get(member)?;
        let definition = top_level_definitions(&file, &resolved)
            .into_iter()
            .find(|d| d.def_id == export.def_id)?;
        let span = definition.span;
        let range = byte_span_to_lsp_range(target, span.start, span.end);
        let hover = crate::hover::declaration_hover(target, &file, span.start).map(|mut hover| {
            hover.range = Some(byte_span_to_lsp_range(
                source,
                token.span.start,
                token.span.end,
            ));
            hover
        });
        return Some(ImportedSymbol {
            location: Location::new(target_uri, range),
            hover,
        });
    }
    None
}

pub fn definition_range_at(source: &str, position: Position) -> Option<Range> {
    let offset = lsp_position_to_byte(source, position);
    let index = NavigationIndex::build(source)?;
    let target = index.target_at(offset)?;
    index
        .definition_span(target)
        .map(|span| byte_span_to_lsp_range(source, span.start, span.end))
}

pub fn references_at(source: &str, position: Position) -> Vec<Range> {
    let offset = lsp_position_to_byte(source, position);
    let Some(index) = NavigationIndex::build(source) else {
        return Vec::new();
    };
    let Some(target) = index.target_at(offset) else {
        return Vec::new();
    };
    index
        .reference_spans(target)
        .into_iter()
        .map(|span| byte_span_to_lsp_range(source, span.start, span.end))
        .collect()
}

pub fn rename_ranges_at(source: &str, position: Position, new_name: &str) -> Option<Vec<Range>> {
    if !is_identifier(new_name) {
        return None;
    }
    let offset = lsp_position_to_byte(source, position);
    let index = NavigationIndex::build(source)?;
    let target = index.target_at(offset)?;
    if matches!(target, NavTarget::Builtin) {
        return None;
    }
    Some(
        index
            .reference_spans(target)
            .into_iter()
            .map(|span| byte_span_to_lsp_range(source, span.start, span.end))
            .collect(),
    )
}

pub fn workspace_symbols_for_document(
    uri: Url,
    source: &str,
    query: &str,
) -> Vec<SymbolInformation> {
    let Some(index) = NavigationIndex::build(source) else {
        return Vec::new();
    };
    let query = query.to_ascii_lowercase();
    top_level_definitions(&index.file, &index.resolved)
        .into_iter()
        .filter(|definition| {
            query.is_empty() || definition.name.to_ascii_lowercase().contains(query.as_str())
        })
        .map(|definition| {
            #[allow(deprecated)]
            SymbolInformation {
                name: definition.name,
                kind: definition.kind,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: byte_span_to_lsp_range(
                        source,
                        definition.span.start,
                        definition.span.end,
                    ),
                },
                container_name: None,
            }
        })
        .collect()
}

struct NavigationIndex {
    file: File,
    resolved: Resolved,
}

impl NavigationIndex {
    fn build(source: &str) -> Option<Self> {
        let tokens = lex(source).ok()?;
        let (file, parse_errors) = parse_file(&tokens);
        if !parse_errors.is_empty() {
            return None;
        }
        let resolved = resolve(&file);
        if !resolved.errors.is_empty() {
            return None;
        }
        Some(Self { file, resolved })
    }

    fn target_at(&self, offset: usize) -> Option<NavTarget> {
        self.binding_at(offset).or_else(|| self.decl_at(offset))
    }

    fn binding_at(&self, offset: usize) -> Option<NavTarget> {
        self.resolved
            .bindings
            .iter()
            .filter(|(span, _)| contains(**span, offset))
            .min_by_key(|(span, _)| span.end.saturating_sub(span.start))
            .map(|(_, binding)| match binding {
                Binding::Decl(id) => NavTarget::Decl(*id),
                Binding::Local(id) => NavTarget::Local(*id),
                Binding::BuiltIn(_) => NavTarget::Builtin,
            })
    }

    fn decl_at(&self, offset: usize) -> Option<NavTarget> {
        top_level_definitions(&self.file, &self.resolved)
            .into_iter()
            .filter(|definition| contains(definition.span, offset))
            .min_by_key(|definition| definition.span.end.saturating_sub(definition.span.start))
            .map(|definition| NavTarget::Decl(definition.def_id))
    }

    fn definition_span(&self, target: NavTarget) -> Option<Span> {
        match target {
            NavTarget::Decl(id) => top_level_definitions(&self.file, &self.resolved)
                .into_iter()
                .find(|definition| definition.def_id == id)
                .map(|definition| definition.span),
            NavTarget::Local(id) => self
                .resolved
                .bindings
                .iter()
                .filter_map(|(span, binding)| match binding {
                    Binding::Local(local_id) if *local_id == id => Some(*span),
                    _ => None,
                })
                .min_by_key(|span| span.start),
            NavTarget::Builtin => None,
        }
    }

    fn reference_spans(&self, target: NavTarget) -> Vec<Span> {
        match target {
            NavTarget::Decl(id) => {
                let mut spans = BTreeSet::<(usize, usize)>::new();
                if let Some(definition) = self.definition_span(target) {
                    spans.insert((definition.start, definition.end));
                }
                for (span, binding) in &self.resolved.bindings {
                    if matches!(binding, Binding::Decl(binding_id) if *binding_id == id) {
                        spans.insert((span.start, span.end));
                    }
                }
                spans
                    .into_iter()
                    .map(|(start, end)| Span::new(start, end))
                    .collect()
            }
            NavTarget::Local(id) => self
                .resolved
                .bindings
                .iter()
                .filter_map(|(span, binding)| match binding {
                    Binding::Local(local_id) if *local_id == id => Some(*span),
                    _ => None,
                })
                .collect(),
            NavTarget::Builtin => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavTarget {
    Decl(DefId),
    Local(LocalId),
    Builtin,
}

struct Definition {
    def_id: DefId,
    name: String,
    kind: SymbolKind,
    span: Span,
}

fn top_level_definitions(file: &File, resolved: &Resolved) -> Vec<Definition> {
    let mut out = Vec::new();
    for decl in &file.decls {
        match decl {
            Decl::Type(TypeDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Type,
                name.span,
                SymbolKind::STRUCT,
            ),
            Decl::Fn(f) => push_definition(
                &mut out,
                resolved,
                &f.name.name,
                DeclKind::Fn,
                f.name.span,
                SymbolKind::FUNCTION,
            ),
            Decl::Store(store) => push_definition(
                &mut out,
                resolved,
                &store.name.name,
                DeclKind::Store,
                store.name.span,
                SymbolKind::STRUCT,
            ),
            Decl::Table(table) => push_definition(
                &mut out,
                resolved,
                &table.record.name.name,
                DeclKind::Type,
                table.record.name.span,
                SymbolKind::STRUCT,
            ),
            // `traces:` defines no name.
            Decl::Traces(_) => {}
            Decl::Database(database) => push_definition(
                &mut out,
                resolved,
                &database.name.name,
                DeclKind::Database,
                database.name.span,
                SymbolKind::MODULE,
            ),
            Decl::Tool(ToolDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Tool,
                name.span,
                SymbolKind::FUNCTION,
            ),
            Decl::Prompt(PromptDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Prompt,
                name.span,
                SymbolKind::FUNCTION,
            ),
            Decl::Agent(AgentDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Agent,
                name.span,
                SymbolKind::FUNCTION,
            ),
            Decl::Eval(EvalDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Eval,
                name.span,
                SymbolKind::METHOD,
            ),
            Decl::Test(TestDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Test,
                name.span,
                SymbolKind::METHOD,
            ),
            Decl::Fixture(FixtureDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Fixture,
                name.span,
                SymbolKind::FUNCTION,
            ),
            Decl::Mock(MockDecl { target, .. }) => push_definition(
                &mut out,
                resolved,
                &target.name,
                DeclKind::Tool,
                target.span,
                SymbolKind::FUNCTION,
            ),
            Decl::Effect(EffectDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Effect,
                name.span,
                SymbolKind::EVENT,
            ),
            Decl::Model(ModelDecl { name, .. }) => push_definition(
                &mut out,
                resolved,
                &name.name,
                DeclKind::Model,
                name.span,
                SymbolKind::CLASS,
            ),
            Decl::Server(server) => push_definition(
                &mut out,
                resolved,
                &server.name.name,
                DeclKind::Server,
                server.name.span,
                SymbolKind::CLASS,
            ),
            Decl::Identity(identity) => push_definition(
                &mut out,
                resolved,
                &identity.name.name,
                DeclKind::Identity,
                identity.name.span,
                SymbolKind::CLASS,
            ),
            // 52g-1: a connector name is not yet a resolvable symbol
            // (its operations become callable in a later slice), so it
            // has no go-to-definition entry yet.
            Decl::Connector(_) => {}
            // Schedule decls reference an existing target by name; the
            // navigation surface defines that target via its own
            // Decl::Agent / Decl::Tool entry, so the schedule itself
            // does not produce a new top-level definition.
            Decl::Import(_) | Decl::Extend(_) | Decl::Schedule(_) => {}
        }
    }
    out
}

fn push_definition(
    out: &mut Vec<Definition>,
    resolved: &Resolved,
    name: &str,
    expected: DeclKind,
    span: Span,
    kind: SymbolKind,
) {
    let Some(def_id) = resolved.symbols.lookup_def(name) else {
        return;
    };
    if resolved.symbols.get(def_id).kind == expected {
        out.push(Definition {
            def_id,
            name: name.to_string(),
            kind,
            span,
        });
    }
}

fn contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_follow_public_declarations_in_the_open_module() {
        let root = std::env::temp_dir().join("corvid-lsp-navigation");
        let main = Url::from_file_path(root.join("main.cor")).unwrap();
        let math = Url::from_file_path(root.join("math.cor")).unwrap();
        let documents = [(math.clone(), "fn hidden() -> Int:\n    return 0\npublic fn twice(n: Int) -> Int:\n    return n * 2\n".into())].into();
        let source = "import \"./math\" as m use twice as double\nagent main() -> Int:\n    return m.twice(double(2))\n";
        for (line, col) in [(0, 25), (0, 35), (2, 14), (2, 21)] {
            let symbol = imported_symbol_at(&documents, &main, source, Position::new(line, col)).unwrap();
            assert_eq!(symbol.location.uri, math);
            assert_eq!(symbol.location.range.start, Position::new(2, 10));
            let hover = symbol.hover.unwrap();
            assert_eq!(hover.range.unwrap().start.line, line);
            assert!(format!("{:?}", hover.contents).contains("fn twice(n: Int) -> Int"));
        }
        let private = source.replace("m.twice(", "m.hidden(");
        assert!(imported_symbol_at(&documents, &main, &private, Position::new(2, 14)).is_none());
        let shadow = "import \"./math\" as m\nagent read(m: Int) -> Int:\n    return m.twice(2)\n";
        assert!(imported_symbol_at(&documents, &main, shadow, Position::new(2, 14)).is_none());
    }

    fn range_text<'a>(source: &'a str, range: Range) -> &'a str {
        let start = offset_for_position(source, range.start);
        let end = offset_for_position(source, range.end);
        &source[start..end]
    }

    fn offset_for_position(source: &str, position: Position) -> usize {
        crate::lsp_position_to_byte(source, position)
    }

    #[test]
    fn finds_definition_for_tool_call() {
        let src = "tool get_order(id: String) -> String\n\nagent run(id: String) -> String:\n    return get_order(id)\n";
        let range = definition_range_at(src, Position { line: 3, character: 13 }).unwrap();
        assert_eq!(range_text(src, range), "get_order");
    }

    #[test]
    fn finds_references_for_local() {
        let src = "agent run(id: String) -> String:\n    order = id\n    return order\n";
        let refs = references_at(src, Position { line: 2, character: 13 });
        let texts = refs
            .into_iter()
            .map(|range| range_text(src, range).to_string())
            .collect::<Vec<_>>();
        assert_eq!(texts, vec!["order".to_string(), "order".to_string()]);
    }

    #[test]
    fn rename_uses_resolver_identity_not_text_search() {
        let src = "tool id(x: String) -> String\n\nagent run(id: String) -> String:\n    return id\n";
        let ranges = rename_ranges_at(src, Position { line: 2, character: 10 }, "ticket_id").unwrap();
        let texts = ranges
            .into_iter()
            .map(|range| range_text(src, range).to_string())
            .collect::<Vec<_>>();
        assert_eq!(texts, vec!["id".to_string(), "id".to_string()]);
    }

    #[test]
    fn rejects_invalid_rename_identifier() {
        let src = "agent run(id: String) -> String:\n    return id\n";
        assert!(rename_ranges_at(src, Position { line: 0, character: 11 }, "not valid").is_none());
    }

    #[test]
    fn workspace_symbols_include_ai_native_decls() {
        let uri = Url::parse("file:///workspace/main.cor").unwrap();
        let src = "effect retrieval:\n    data: grounded\n\nmodel haiku:\n    capability: basic\n\nprompt answer(x: String) -> String:\n    \"ok\"\n";
        let symbols = workspace_symbols_for_document(uri, src, "");
        let names = symbols
            .into_iter()
            .map(|symbol| symbol.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["retrieval", "haiku", "answer"]);
    }
}
