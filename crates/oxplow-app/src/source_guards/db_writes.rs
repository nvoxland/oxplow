//! What a thin caller may not call (tsk903): every function in oxplow-db
//! that reaches a database write, and every consent or credential call.
//! The list is clippy's `disallowed-methods` in each thin caller's
//! `clippy.toml`, so the check is the compiler's own name resolution — an
//! alias, a store built in place, a trait path or a turbofish is the same
//! call — and a write is told by what its body does, never by its name.
//!
//! A function writes when its body runs a statement that changes the
//! database (`execute`, `execute_batch`, `commit`) or opens a write
//! transaction (`transaction`, `rehearse`), or calls a function of
//! oxplow-db that writes: its own type's methods (`self.f`, `Self::f`), a
//! type's (`Type::f`), a free function by name. A trait method writes when
//! one of oxplow-db's implementations of it does. `Database::read` only
//! opens a deferred transaction it always rolls back, so a read through it
//! is a read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::visit::Visit;

/// The calls that change the database themselves.
const WRITES: [&str; 5] = [
    "execute",
    "execute_batch",
    "commit",
    "transaction",
    "rehearse",
];

/// Types whose statements change only their own connection's session —
/// a read's temp tables and views and its `query_only` pragma, each
/// undone when the guard drops — never what the database keeps.
const SESSION_ONLY: [&str; 3] = ["ReadSession", "TempTables", "TempViews"];

/// Why a thin caller may not call a database write.
pub const DB_REASON: &str =
    "a thin caller's database write is a command on the bus, or a service's own ingest (.context/ipc-and-stores.md)";

/// Why a thin caller may not touch consent or credentials.
pub const CONSENT_REASON: &str =
    "consent and credentials change only inside oxplow-app, behind a person's confirmation";

/// What a function body calls, as far as its syntax tree tells.
#[derive(Default)]
struct Calls {
    /// `self.f(..)` and `Self::f(..)`.
    own: BTreeSet<String>,
    /// `f(..)` or `module::f(..)`, and `f` passed as a value.
    free: BTreeSet<String>,
    /// `Type::f(..)`.
    assoc: BTreeSet<(String, String)>,
    /// `<expr>.f(..)` on anything but `self`.
    method: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let name = m.method.to_string();
        match &*m.receiver {
            syn::Expr::Path(p) if p.path.is_ident("self") => self.own.insert(name),
            _ => self.method.insert(name),
        };
        syn::visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
        let segs: Vec<String> = p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        match segs.as_slice() {
            [.., ty, f] if ty == "Self" => {
                self.own.insert(f.clone());
            }
            [.., ty, f] if ty.starts_with(char::is_uppercase) => {
                self.assoc.insert((ty.clone(), f.clone()));
            }
            [.., f] if !f.starts_with(char::is_uppercase) => {
                self.free.insert(f.clone());
            }
            _ => {}
        }
        syn::visit::visit_expr_path(self, p);
    }
}

/// Where a function is defined.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Owner {
    /// A free function in a module.
    Free,
    /// A method of `impl <type>` — or `impl <trait> for <type>`.
    Impl { ty: String, tr: Option<String> },
    /// A trait's own (default or declared) method.
    Trait(String),
}

struct Function {
    owner: Owner,
    name: String,
    /// `crate::module` it is defined in.
    module: String,
    public: bool,
    calls: Calls,
}

/// Every function in `crate_name`'s `src/`, outside its test code.
fn functions(crate_dir: &Path, crate_name: &str) -> Vec<Function> {
    let mut out = Vec::new();
    let src = crate_dir.join("src");
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let rel = path.strip_prefix(&src).unwrap().with_extension("");
            let mut segs: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            if matches!(
                segs.last().map(String::as_str),
                Some("lib" | "main" | "mod")
            ) {
                segs.pop();
            }
            let module = std::iter::once(crate_name.to_string())
                .chain(segs)
                .collect::<Vec<_>>()
                .join("::");
            let text = std::fs::read_to_string(&path).unwrap();
            let file = syn::parse_file(&text)
                .unwrap_or_else(|e| panic!("{} doesn't parse: {e}", path.display()));
            collect(&file.items, &module, &mut out);
        }
    }
    out
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Meta>()
                .is_ok_and(|m| m.path().is_ident("test"))
    })
}

fn last_ident(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

fn body_calls(block: &syn::Block) -> Calls {
    let mut calls = Calls::default();
    calls.visit_block(block);
    calls
}

fn collect(items: &[syn::Item], module: &str, out: &mut Vec<Function>) {
    for item in items {
        match item {
            syn::Item::Fn(f) if !is_test(&f.attrs) => out.push(Function {
                owner: Owner::Free,
                name: f.sig.ident.to_string(),
                module: module.to_string(),
                public: matches!(f.vis, syn::Visibility::Public(_)),
                calls: body_calls(&f.block),
            }),
            syn::Item::Impl(i) if !is_test(&i.attrs) => {
                let syn::Type::Path(ty) = &*i.self_ty else {
                    continue;
                };
                let tr = i.trait_.as_ref().map(|(_, p, _)| last_ident(p));
                for item in &i.items {
                    if let syn::ImplItem::Fn(f) = item {
                        out.push(Function {
                            owner: Owner::Impl {
                                ty: last_ident(&ty.path),
                                tr: tr.clone(),
                            },
                            name: f.sig.ident.to_string(),
                            module: module.to_string(),
                            public: tr.is_some() || matches!(f.vis, syn::Visibility::Public(_)),
                            calls: body_calls(&f.block),
                        });
                    }
                }
            }
            syn::Item::Trait(t) if !is_test(&t.attrs) => {
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item {
                        out.push(Function {
                            owner: Owner::Trait(t.ident.to_string()),
                            name: f.sig.ident.to_string(),
                            module: module.to_string(),
                            public: matches!(t.vis, syn::Visibility::Public(_)),
                            calls: f.default.as_ref().map(body_calls).unwrap_or_default(),
                        });
                    }
                }
            }
            syn::Item::Mod(m) if !is_test(&m.attrs) => {
                if let Some((_, items)) = &m.content {
                    collect(items, &format!("{module}::{}", m.ident), out);
                }
            }
            _ => {}
        }
    }
}

/// Which of `fns` write, to a fixpoint: `writes[i]` for `fns[i]`.
fn classify(fns: &[Function]) -> Vec<bool> {
    let session_only = |f: &Function| matches!(&f.owner, Owner::Impl { ty, .. } if SESSION_ONLY.contains(&ty.as_str()));
    let mut writes: Vec<bool> = fns
        .iter()
        .map(|f| !session_only(f) && f.calls.method.iter().any(|m| WRITES.contains(&m.as_str())))
        .collect();
    loop {
        let free: BTreeSet<&str> = fns
            .iter()
            .zip(&writes)
            .filter(|(f, w)| **w && f.owner == Owner::Free)
            .map(|(f, _)| f.name.as_str())
            .collect();
        // (type, method) pairs that write, whichever impl block they are in.
        let typed: BTreeSet<(&str, &str)> = fns
            .iter()
            .zip(&writes)
            .filter(|(_, w)| **w)
            .filter_map(|(f, _)| match &f.owner {
                Owner::Impl { ty, .. } => Some((ty.as_str(), f.name.as_str())),
                Owner::Trait(tr) => Some((tr.as_str(), f.name.as_str())),
                Owner::Free => None,
            })
            .collect();
        let mut changed = false;
        for (i, f) in fns.iter().enumerate() {
            if writes[i] || session_only(f) {
                continue;
            }
            let own_ty = match &f.owner {
                Owner::Impl { ty, .. } => Some(ty.as_str()),
                Owner::Trait(tr) => Some(tr.as_str()),
                Owner::Free => None,
            };
            let hit = f.calls.free.iter().any(|n| free.contains(n.as_str()))
                || f.calls
                    .assoc
                    .iter()
                    .any(|(t, n)| typed.contains(&(t.as_str(), n.as_str())))
                || own_ty
                    .is_some_and(|t| f.calls.own.iter().any(|n| typed.contains(&(t, n.as_str()))));
            if hit {
                writes[i] = true;
                changed = true;
            }
        }
        // A trait method writes when an implementation of it does.
        for (i, f) in fns.iter().enumerate() {
            if let Owner::Trait(tr) = &f.owner {
                let implemented = fns.iter().zip(&writes).any(|(g, w)| {
                    *w && g.name == f.name
                        && matches!(&g.owner, Owner::Impl { tr: Some(t), .. } if t == tr)
                });
                if implemented && !writes[i] {
                    writes[i] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            return writes;
        }
    }
}

/// The paths a thin caller may not call, each with its reason, sorted.
pub fn denied(root: &Path) -> Vec<(String, &'static str)> {
    let db = functions(&root.join("crates/oxplow-db"), "oxplow_db");
    let writes = classify(&db);
    let mut out: BTreeMap<String, &'static str> = BTreeMap::new();
    // Traits oxplow-db implements, by name, where they are defined.
    let mut traits: BTreeMap<String, String> = BTreeMap::new();
    for krate in ["oxplow-domain", "oxplow-db"] {
        let name = krate.replace('-', "_");
        for f in functions(&root.join("crates").join(krate), &name) {
            if let (Owner::Trait(tr), true) = (&f.owner, f.public) {
                traits.insert(tr.clone(), f.module.clone());
            }
        }
    }
    for (f, w) in db.iter().zip(&writes) {
        if !*w || !f.public {
            continue;
        }
        let path = match &f.owner {
            Owner::Free => format!("{}::{}", f.module, f.name),
            Owner::Impl { ty, tr: None } => format!("{}::{ty}::{}", f.module, f.name),
            Owner::Impl { tr: Some(tr), .. } | Owner::Trait(tr) => match traits.get(tr) {
                Some(module) => format!("{module}::{tr}::{}", f.name),
                // Another crate's trait (`Drop`, `From`): not a store's.
                None => continue,
            },
        };
        out.insert(path, DB_REASON);
    }
    // Consent and credentials, every call.
    for (krate, module, owner) in [
        ("oxplow-app", "oxplow_app::exec_consent", "ApprovalStore"),
        ("oxplow-ai", "oxplow_ai::secrets", "SecretStore"),
    ] {
        let name = krate.replace('-', "_");
        for f in functions(&root.join("crates").join(krate), &name) {
            let mine = match &f.owner {
                Owner::Impl { ty, tr: None } => ty == owner && f.public,
                Owner::Trait(tr) => tr == owner,
                _ => false,
            };
            if mine && f.module == module {
                out.insert(format!("{module}::{owner}::{}", f.name), CONSENT_REASON);
            }
        }
    }
    out.into_iter().collect()
}
