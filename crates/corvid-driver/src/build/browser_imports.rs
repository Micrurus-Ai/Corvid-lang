use super::FrontendBundle;
use corvid_ast::{Decl, File, ImportSource};
use std::path::Path;

/// Check the browser entry and its resolved dependencies before emitting public assets.
pub(super) fn check(frontend: &FrontendBundle, entry: &Path) -> anyhow::Result<()> {
    check_module(&frontend.file, &frontend.source, entry)?;
    if let Some(graph) = &frontend.modules {
        let mut modules: Vec<_> = graph.all_modules.values().collect();
        modules.sort_by_key(|module| &module.path);
        for module in modules {
            check_module(&module.file, &module.source, &module.path)?;
        }
    }
    Ok(())
}

fn check_module(file: &File, source: &str, path: &Path) -> anyhow::Result<()> {
    for decl in &file.decls {
        let Decl::Import(import) = decl else { continue };
        let module = import.module.strip_suffix(".cor").unwrap_or(&import.module);
        if import.source == ImportSource::Python
            || matches!(module, "std:env" | "std:secrets" | "std:io" | "std:db")
        {
            let line = 1 + source[..import.span.start]
                .bytes()
                .filter(|&byte| byte == b'\n')
                .count();
            anyhow::bail!(
                "{}:{line}: browser handlers cannot import server-only module {}",
                path.display(),
                import.module
            );
        }
    }
    Ok(())
}
