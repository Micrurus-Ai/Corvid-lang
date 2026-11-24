//! Records the resolved type of every type reference IR lowering
//! reads: parameters, return types, record and variant fields, and
//! route inputs and outputs.
//!
//! The checker already resolves most of these while checking, and
//! `type_ref_to_type` records each result in `type_refs`. This sweep
//! guarantees the table is complete even for declarations the checker
//! never needs to look inside, such as a record no code constructs.
//! IR lowering treats a missing entry as a compiler bug, so a lowered
//! position added without a matching line here fails loudly in tests.

use super::Checker;
use corvid_ast::{Decl, ExtendMethodKind, File, Param, TypeRef};

impl<'a> Checker<'a> {
    pub(super) fn record_signature_types(&mut self, file: &File) {
        // Each position was checked already; resolving it again must
        // not repeat its diagnostics.
        let mark = self.errors.len();
        for decl in &file.decls {
            match decl {
                Decl::Type(t) => {
                    for field in t.fields.iter().chain(t.variants.iter().flat_map(|v| v.fields.iter())) {
                        self.record(&field.ty);
                    }
                }
                Decl::Tool(t) => self.record_signature(&t.params, &t.return_ty),
                Decl::Prompt(p) => self.record_signature(&p.params, &p.return_ty),
                Decl::Agent(a) => self.record_signature(&a.params, &a.return_ty),
                Decl::Fn(f) => self.record_signature(&f.params, &f.return_ty),
                Decl::Fixture(f) => self.record_signature(&f.params, &f.return_ty),
                Decl::Mock(m) => self.record_signature(&m.params, &m.return_ty),
                Decl::Connector(c) => {
                    for op in &c.operations {
                        self.record_signature(&op.params, &op.return_ty);
                    }
                }
                Decl::Server(s) => {
                    for route in &s.routes {
                        for param in &route.path_params {
                            self.record(&param.ty);
                        }
                        if let Some(query) = &route.query_ty {
                            self.record(query);
                        }
                        if let Some(body) = &route.body_ty {
                            self.record(body);
                        }
                        self.record(&route.response.ty);
                    }
                }
                Decl::Extend(ext) => {
                    for method in &ext.methods {
                        match &method.kind {
                            ExtendMethodKind::Tool(t) => self.record_signature(&t.params, &t.return_ty),
                            ExtendMethodKind::Prompt(p) => self.record_signature(&p.params, &p.return_ty),
                            ExtendMethodKind::Agent(a) => self.record_signature(&a.params, &a.return_ty),
                        }
                    }
                }
                Decl::Import(_)
                | Decl::Effect(_)
                | Decl::Model(_)
                | Decl::Eval(_)
                | Decl::Test(_)
                | Decl::Schedule(_)
                | Decl::Store(_)
                | Decl::Identity(_) => {}
            }
        }
        self.errors.truncate(mark);
    }

    fn record_signature(&mut self, params: &[Param], return_ty: &TypeRef) {
        for param in params {
            self.record(&param.ty);
        }
        self.record(return_ty);
    }

    fn record(&mut self, ty: &TypeRef) {
        let _ = self.type_ref_to_type(ty);
    }
}
