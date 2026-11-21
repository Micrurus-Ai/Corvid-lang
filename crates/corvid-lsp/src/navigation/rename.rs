use super::*;
use corvid_resolve::{collect_public_exports, resolve_import_path};
use corvid_syntax::TokKind;
use lsp_types::{TextEdit, WorkspaceEdit};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Documents = HashMap<Url, String>;

fn file_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

fn project_files(dir: &Path, files: &mut HashMap<PathBuf, Url>) -> Option<()> {
    for entry in dir.read_dir().ok()? {
        let entry = entry.ok()?;
        let ty = entry.file_type().ok()?;
        let path = entry.path();
        if ty.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some("target" | ".git" | "node_modules" | ".venv")
            ) {
                project_files(&path, files)?;
            }
        } else if ty.is_file() && path.extension().is_some_and(|e| e == "cor") {
            files
                .entry(file_key(&path))
                .or_insert(Url::from_file_path(&path).ok()?);
        }
    }
    Some(())
}

/// Reject edits that change name resolution, for example when the new
/// spelling would capture a local parameter in one consumer.
fn checked_edits(
    source: &str,
    index: &NavigationIndex,
    spans: Vec<Span>,
    new: &str,
) -> Option<Vec<TextEdit>> {
    let mut spans = spans;
    spans.sort_by_key(|s| (s.start, s.end));
    spans.dedup();
    let mut rewritten = source.to_owned();
    for span in spans.iter().rev() {
        rewritten.replace_range(span.start..span.end, new);
    }
    let after = NavigationIndex::build(&rewritten)?;
    let bindings = |index: &NavigationIndex| {
        let mut pairs: Vec<_> = index.resolved.bindings.iter().collect();
        pairs.sort_by_key(|(span, _)| (span.start, span.end));
        pairs
            .into_iter()
            .map(|(_, binding)| binding.clone())
            .collect::<Vec<_>>()
    };
    if bindings(index) != bindings(&after) {
        return None;
    }
    Some(
        spans
            .into_iter()
            .map(|s| TextEdit {
                range: byte_span_to_lsp_range(source, s.start, s.end),
                new_text: new.into(),
            })
            .collect(),
    )
}

fn consumer_spans(source: &str, path: &Path, owner: &Path, name: &str) -> Option<Vec<Span>> {
    let tokens = lex(source).ok()?;
    let (file, _) = parse_file(&tokens);
    let resolved = resolve(&file);
    let mut spans = Vec::new();
    for decl in &file.decls {
        let Decl::Import(import) = decl else { continue };
        if import.source != corvid_ast::ImportSource::Corvid
            || file_key(&resolve_import_path(path, &import.module)) != owner
        {
            continue;
        }
        for item in &import.use_items {
            if item.name.name != name {
                continue;
            }
            spans.push(item.name.span);
            if item.alias.is_none() {
                let id = resolved.symbols.lookup_def(name)?;
                spans.extend(
                    resolved
                        .bindings
                        .iter()
                        .filter_map(|(s, b)| (*b == Binding::Decl(id)).then_some(*s)),
                );
            }
        }
        if let Some(alias) = &import.alias {
            let id = resolved.symbols.lookup_def(&alias.name)?;
            for parts in tokens.windows(3) {
                if matches!(parts[1].kind, TokKind::Dot)
                    && matches!(&parts[2].kind, TokKind::Ident(n) if n == name)
                    && resolved.bindings.get(&parts[0].span) == Some(&Binding::Decl(id))
                {
                    spans.push(parts[2].span);
                }
            }
        }
    }
    Some(spans)
}

pub(crate) fn workspace_rename(
    docs: &Documents,
    uri: &Url,
    position: Position,
    new: &str,
    roots: &[PathBuf],
) -> Option<WorkspaceEdit> {
    if !is_identifier(new) || TokKind::keyword_from(new).is_some() {
        return None;
    }
    let source = docs.get(uri)?;
    let index = NavigationIndex::build(source)?;
    let offset = lsp_position_to_byte(source, position);
    let target = index.target_at(offset);
    let mut changes = HashMap::new();
    let edit = |changes| {
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    };

    // An explicit module or selective alias has its own local identity.
    for decl in &index.file.decls {
        let Decl::Import(import) = decl else { continue };
        for alias in import
            .alias
            .iter()
            .chain(import.use_items.iter().filter_map(|i| i.alias.as_ref()))
        {
            let id = index.resolved.symbols.lookup_def(&alias.name)?;
            if contains(alias.span, offset) || target == Some(NavTarget::Decl(id)) {
                let mut spans = index.reference_spans(NavTarget::Decl(id));
                spans.push(alias.span);
                changes.insert(uri.clone(), checked_edits(source, &index, spans, new)?);
                return edit(changes);
            }
        }
    }
    let imported = imported_symbol_at(docs, uri, source, position);
    let (owner_uri, owner_source, owner_index, owner_id) = if let Some(imported) = imported {
        let location = imported.location;
        let text = match docs.get(&location.uri) {
            Some(text) => text.clone(),
            None => std::fs::read_to_string(location.uri.to_file_path().ok()?).ok()?,
        };
        let index = NavigationIndex::build(&text)?;
        let pos = lsp_position_to_byte(&text, location.range.start);
        let NavTarget::Decl(id) = index.decl_at(pos)? else {
            return None;
        };
        (location.uri, text, index, id)
    } else {
        let target = target?;
        let public = collect_public_exports(&index.file, &index.resolved);
        if let Some(export) = public
            .values()
            .find(|e| target == NavTarget::Decl(e.def_id))
        {
            (uri.clone(), source.clone(), index, export.def_id)
        } else {
            if target == NavTarget::Builtin
                || matches!(target, NavTarget::Decl(id)
                if matches!(index.resolved.symbols.get(id).kind, DeclKind::Import | DeclKind::ImportedUse))
            {
                return None;
            }
            changes.insert(
                uri.clone(),
                checked_edits(source, &index, index.reference_spans(target), new)?,
            );
            return edit(changes);
        }
    };
    let owner_path = owner_uri.to_file_path().ok()?;
    let owner = file_key(&owner_path);
    let name = &owner_index.resolved.symbols.get(owner_id).name;
    changes.insert(
        owner_uri,
        checked_edits(
            &owner_source,
            &owner_index,
            owner_index.reference_spans(NavTarget::Decl(owner_id)),
            new,
        )?,
    );
    let mut files = HashMap::new();
    for uri in docs.keys() {
        if let Ok(path) = uri.to_file_path() {
            files.insert(file_key(&path), uri.clone());
        }
    }
    if roots.is_empty() {
        if let Some(project) = owner_path
            .ancestors()
            .find(|dir| dir.join("corvid.toml").is_file())
        {
            project_files(project, &mut files)?;
        }
    } else {
        for root in roots {
            project_files(root, &mut files)?;
        }
    }
    for (path, uri) in files {
        if path == owner {
            continue;
        }
        let disk;
        let text = match docs.get(&uri) {
            Some(text) => text,
            None => {
                disk = std::fs::read_to_string(&path).ok()?;
                &disk
            }
        };
        if !text.contains(name) || !text.contains("import") {
            continue;
        }
        let spans = consumer_spans(text, &path, &owner, name)?;
        if !spans.is_empty() {
            let index = NavigationIndex::build(text)?;
            changes.insert(uri, checked_edits(text, &index, spans, new)?);
        }
    }
    edit(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_rename_follows_bindings_and_refuses_capture() {
        let dir = std::env::temp_dir().join("corvid-rename-buffers");
        let owner = Url::from_file_path(dir.join("math.cor")).unwrap();
        let main = Url::from_file_path(dir.join("main.cor")).unwrap();
        let caller = Url::from_file_path(dir.join("caller.cor")).unwrap();
        let mut docs: Documents = [
            (owner.clone(), "public fn double(n: Int) -> Int:\n    return n * 2\n".into()),
            (main.clone(), "import \"./math\" as m use double as twice\nagent main() -> Int:\n    return m.double(twice(2))\n".into()),
            (caller.clone(), "import \"./math\" use double\nagent test_it(scale: Int) -> Int:\n    return double(scale)\n".into()),
        ].into();
        let qualified = Position::new(2, 15);
        assert!(workspace_rename(&docs, &main, qualified, "scale", &[]).is_none());
        assert!(workspace_rename(&docs, &main, qualified, "return", &[]).is_none());
        let edits = workspace_rename(&docs, &main, qualified, "multiply", &[])
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            (
                edits[&owner].len(),
                edits[&main].len(),
                edits[&caller].len()
            ),
            (1, 2, 2)
        );
        for (uri, edits) in &edits {
            for edit in edits {
                let text = &docs[uri];
                assert_eq!(
                    &text[lsp_position_to_byte(text, edit.range.start)
                        ..lsp_position_to_byte(text, edit.range.end)],
                    "double"
                );
            }
        }
        let edits = workspace_rename(&docs, &main, Position::new(2, 22), "again", &[])
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[&main].len(), 2);
        docs.insert(caller.clone(), "import \"./math\" use double\nagent use_local(double: Int) -> Int:\n    return double\n".into());
        let edits = workspace_rename(&docs, &owner, Position::new(0, 12), "multiply", &[])
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits[&caller].len(), 1);
    }
}
