//! Symbol table, scope stack, and binding types.

use corvid_ast::Span;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Stable ID of a top-level declaration within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DefId(pub u32);

/// Stable ID of a local binding (parameter or `x = ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LocalId(pub u32);

/// A reference produced by the resolver for each identifier use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binding {
    /// Refers to a top-level declaration.
    Decl(DefId),
    /// Refers to a local binding (parameter or assignment result).
    Local(LocalId),
    /// Refers to a built-in (type name or language-level sentinel).
    BuiltIn(BuiltIn),
}

/// Names that are always in scope without a user declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltIn {
    // Primitive types.
    Int,
    Float,
    String,
    Bool,
    Nothing,
    List,
    /// `Map<K, V>` (slice 45g).
    Map,
    Stream,
    Result,
    Option,
    Weak,
    Partial,
    ResumeToken,
    Ok,
    Err,
    Some,
    None,
    Grounded,
    WeakNew,
    /// `range(start, end) -> List<Int>` (slice 45f).
    Range,
    WeakUpgrade,
    StreamMerge,
    Resume,
    StreamResumeToken,
    Ask,
    Choose,
    /// Phase 33S3a — `DbHandle` is an opaque, refcounted primitive
    /// type produced ONLY by the executing `db_open` stdlib tool
    /// (see `std/db.cor`). Registered as a builtin so user code can
    /// name it in agent signatures (`agent f() -> DbHandle: ...`)
    /// without resolving as `UndefinedName`. The typechecker maps
    /// the name to `Type::DbHandle`; the VM-value layer maps the
    /// dispatched return to `Value::DbHandle(Arc<DbHandleInner>)`.
    /// Together these make the opacity of the SQLite-connection
    /// handle a load-bearing language property.
    DbHandle,
    /// Phase 33R5b-a — `JsonValue` is an opaque, refcounted parsed
    /// JSON payload produced by `std/json.cor`'s `json_parse` and
    /// threaded through the typed accessor tools. Wraps
    /// `Arc<serde_json::Value>` at the VM layer; the typed
    /// accessors return `Result<T, String>` so field-type
    /// mismatches surface as recoverable errors. Unlike DbHandle,
    /// JsonValue has no opacity gate at `json_to_value` because
    /// the payload IS the JSON shape — there is no underlying
    /// registry the value indexes into.
    JsonValue,
    /// Phase 33R5b-a — `JsonBuilder` is an opaque, mutable JSON
    /// object builder. Wraps
    /// `Arc<Mutex<serde_json::Map<String, serde_json::Value>>>`
    /// at the VM layer; `json_object_set_*` mutates the inner
    /// map and returns the same builder for fluent chaining;
    /// `json_object_finish` snapshots and serialises without
    /// invalidating the builder (so set+finish cycles can
    /// continue).
    JsonBuilder,
    // Structural sentinels (surface as Idents today; real variants later).
    Break,
    Continue,
    Pass,
}

/// Kind of top-level declaration, for error messages and later passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclKind {
    Import,
    /// One variant of a sum type (slice 45h). The variant is a
    /// file-scope constructor; `variant_owners` in `Resolved` maps
    /// it back to its owning type and index.
    Variant,
    ImportedUse,
    Type,
    Store,
    Tool,
    Prompt,
    Agent,
    Eval,
    Test,
    Fixture,
    Mock,
    Effect,
    /// `model Name:` catalog entry (Phase 20h typed model substrate).
    Model,
    /// `server Name:` backend route surface.
    Server,
}

/// An entry in the file-level symbol table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclEntry {
    pub id: DefId,
    pub name: String,
    pub kind: DeclKind,
    pub span: Span,
}

/// File-level symbol table. Populated in resolver pass 1.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    entries: Vec<DeclEntry>,
    by_name: HashMap<String, DefId>,
    builtins: HashMap<String, BuiltIn>,
}

impl SymbolTable {
    pub fn new() -> Self {
        let mut t = SymbolTable::default();
        t.register_builtins();
        t
    }

    fn register_builtins(&mut self) {
        self.builtins.insert("Int".into(), BuiltIn::Int);
        self.builtins.insert("Float".into(), BuiltIn::Float);
        self.builtins.insert("String".into(), BuiltIn::String);
        self.builtins.insert("Bool".into(), BuiltIn::Bool);
        self.builtins.insert("Nothing".into(), BuiltIn::Nothing);
        self.builtins.insert("List".into(), BuiltIn::List);
        self.builtins.insert("Map".into(), BuiltIn::Map);
        self.builtins.insert("Stream".into(), BuiltIn::Stream);
        self.builtins.insert("Result".into(), BuiltIn::Result);
        self.builtins.insert("Option".into(), BuiltIn::Option);
        self.builtins.insert("Weak".into(), BuiltIn::Weak);
        self.builtins.insert("Partial".into(), BuiltIn::Partial);
        self.builtins
            .insert("ResumeToken".into(), BuiltIn::ResumeToken);
        self.builtins.insert("Grounded".into(), BuiltIn::Grounded);
        self.builtins.insert("Ok".into(), BuiltIn::Ok);
        self.builtins.insert("Err".into(), BuiltIn::Err);
        self.builtins.insert("Some".into(), BuiltIn::Some);
        self.builtins.insert("None".into(), BuiltIn::None);
        self.builtins.insert("Weak::new".into(), BuiltIn::WeakNew);
        self.builtins.insert("range".into(), BuiltIn::Range);
        self.builtins
            .insert("Weak::upgrade".into(), BuiltIn::WeakUpgrade);
        self.builtins.insert("merge".into(), BuiltIn::StreamMerge);
        self.builtins.insert("resume".into(), BuiltIn::Resume);
        self.builtins
            .insert("resume_token".into(), BuiltIn::StreamResumeToken);
        self.builtins.insert("ask".into(), BuiltIn::Ask);
        self.builtins.insert("choose".into(), BuiltIn::Choose);
        self.builtins.insert("break".into(), BuiltIn::Break);
        self.builtins.insert("continue".into(), BuiltIn::Continue);
        // Phase 33S3a — see the `BuiltIn::DbHandle` docstring.
        self.builtins.insert("DbHandle".into(), BuiltIn::DbHandle);
        // Phase 33R5b-a — see the `BuiltIn::JsonValue` /
        // `BuiltIn::JsonBuilder` docstrings.
        self.builtins.insert("JsonValue".into(), BuiltIn::JsonValue);
        self.builtins
            .insert("JsonBuilder".into(), BuiltIn::JsonBuilder);
        self.builtins.insert("pass".into(), BuiltIn::Pass);
    }

    /// Insert a top-level declaration.
    ///
    /// Returns `Ok(DefId)` on success. On duplicate, returns `Err(first_span)`
    /// — the caller records the duplicate error and proceeds.
    pub fn declare(&mut self, name: &str, kind: DeclKind, span: Span) -> Result<DefId, Span> {
        if let Some(existing_id) = self.by_name.get(name) {
            let existing = &self.entries[existing_id.0 as usize];
            return Err(existing.span);
        }
        let id = DefId(self.entries.len() as u32);
        self.entries.push(DeclEntry {
            id,
            name: name.to_string(),
            kind,
            span,
        });
        self.by_name.insert(name.to_string(), id);
        Ok(id)
    }

    /// Insert or replace a top-level declaration. If a declaration with
    /// the same name already exists, it is replaced: the old entry is
    /// updated in place and the same `DefId` is reused. Returns the
    /// `DefId` and whether a replacement occurred (with the old entry).
    pub fn declare_or_replace(
        &mut self,
        name: &str,
        kind: DeclKind,
        span: Span,
    ) -> (DefId, Option<DeclEntry>) {
        if let Some(&existing_id) = self.by_name.get(name) {
            let old = self.entries[existing_id.0 as usize].clone();
            self.entries[existing_id.0 as usize] = DeclEntry {
                id: existing_id,
                name: name.to_string(),
                kind,
                span,
            };
            (existing_id, Some(old))
        } else {
            let id = DefId(self.entries.len() as u32);
            self.entries.push(DeclEntry {
                id,
                name: name.to_string(),
                kind,
                span,
            });
            self.by_name.insert(name.to_string(), id);
            (id, None)
        }
    }

    /// Allocate a fresh `DefId` for a declaration that lives in a
    /// scoped table (NOT the file-level by-name namespace). Used for
    /// Methods inside `extend T:` blocks — they share names
    /// across types (`Point.distance`, `Line.distance`) so they can't
    /// go in the global by-name table, but they still need stable
    /// identity for downstream IR + diagnostics. Caller is responsible
    /// for storing the (scope, name) → DefId mapping in their own
    /// side table.
    pub fn allocate_def(&mut self, name: &str, kind: DeclKind, span: Span) -> DefId {
        let id = DefId(self.entries.len() as u32);
        self.entries.push(DeclEntry {
            id,
            name: name.to_string(),
            kind,
            span,
        });
        id
    }

    pub fn lookup(&self, name: &str) -> Option<Binding> {
        if let Some(&id) = self.by_name.get(name) {
            return Some(Binding::Decl(id));
        }
        if let Some(&b) = self.builtins.get(name) {
            return Some(Binding::BuiltIn(b));
        }
        None
    }

    /// Look up the `DefId` for a top-level declaration by name.
    pub fn lookup_def(&self, name: &str) -> Option<DefId> {
        self.by_name.get(name).copied()
    }

    pub fn entries(&self) -> &[DeclEntry] {
        &self.entries
    }

    pub fn get(&self, id: DefId) -> &DeclEntry {
        &self.entries[id.0 as usize]
    }
}

/// Lexical scope of local bindings (parameters and `x = ...`).
#[derive(Debug, Clone, Default)]
pub struct LocalScope {
    locals: HashMap<String, LocalId>,
}

impl LocalScope {
    pub fn insert(&mut self, name: &str, id: LocalId) {
        // Shadowing allowed: later insertions overwrite earlier ones.
        self.locals.insert(name.to_string(), id);
    }

    pub fn lookup(&self, name: &str) -> Option<LocalId> {
        self.locals.get(name).copied()
    }
}
