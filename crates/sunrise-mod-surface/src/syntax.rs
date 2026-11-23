//! What the parser reads of one source file for [`crate::uses`]: where structs are built and
//! matched, which paths are written, which impl each `Self` stands in, what the file defines,
//! and the byte ranges of macro bodies, whose tokens the parser does not see into.
use crate::scan::Token;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use syn::visit::{self, Visit};

/// How a struct is built or matched at one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum How {
    /// A literal or pattern that lists every field: `S { a, b }`, `S(a, b)`.
    Full,
    /// A literal that ends in `..` or a base: `S { a, ..d }`.
    Base,
    /// A name used as a value or bound in a pattern by itself (`.map(Pos)`, `Marker`): a build
    /// only for a tuple or unit struct.
    Value,
}

/// A place a struct may be built: the path written, as names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Build<'t> {
    /// The path's names; `Self` for `Self { … }`.
    pub path: Vec<&'t str>,
    /// Written with a leading `::`.
    pub rooted: bool,
    pub how: How,
    /// A brace literal (`S { … }`), for the `..Default::default()` advice.
    pub brace: bool,
    pub line: usize,
    /// The byte where the path starts.
    pub byte: usize,
    /// The form of the candidate, if no lexical binding obscures its path.
    pub variant: Option<VariantUse>,
}

/// The syntax that a local variant must support to explain a build candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VariantUse {
    Braced,
    Tuple,
    Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VariantShape {
    Named,
    Tuple,
    Unit,
}

#[derive(Debug)]
pub(crate) struct Variant {
    /// Unknown if conditional or repeated.
    pub shape: Option<VariantShape>,
    /// The tuple-field scan only needs an unconditional tuple declaration.
    pub has_tuple: bool,
}

/// A tuple struct's field read by its position where the parser can tie the read to the
/// struct's path ([`Reader::tie`]): `X(..).0`, `v.0` where `v` is bound by `let v: X`,
/// `let v = X(..)` or a parameter `v: X` (through `&` and `&mut`), `self.0` in a method (path
/// `Self`), and each position a pattern `X(a, b, ..)` lists before its `..`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TupleField<'t> {
    pub tie: Tie<'t>,
    pub position: u32,
    pub line: usize,
    /// The byte where the read is.
    pub byte: usize,
}

/// An `impl` block: the bytes of its body, and the path of the type it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Impl<'t> {
    pub body: Range<usize>,
    /// Its names, an empty first one for a leading `::`; `None` when the type is not a plain
    /// path the scan can read (a tuple, a macro), so what `Self` means there is unknown.
    pub target: Option<Vec<&'t str>>,
    /// A lexical type name shadows the target's qualifier at the impl header.
    pub shadowed_target: bool,
}

/// The evidence a single item supplies about a name in a module or block.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Binding<'t> {
    Enum,
    Module,
    VariantImport { owner: &'t str, variant: &'t str },
    Other,
}

/// A scope's bindings, including conditional items, which can defeat but not prove a binding.
#[derive(Debug, Default)]
pub(crate) struct ScopeNames<'t> {
    types: BTreeMap<&'t str, Binding<'t>>,
    values: BTreeMap<&'t str, Binding<'t>>,
    pub glob: bool,
}

impl<'t> ScopeNames<'t> {
    pub fn get(&self, space: Namespace, name: &str) -> Option<Binding<'t>> {
        match space {
            Namespace::Type => self.types.get(name),
            Namespace::Value => self.values.get(name),
        }
        .copied()
    }

    fn insert(&mut self, space: Namespace, name: &'t str, binding: Binding<'t>) {
        let names = match space {
            Namespace::Type => &mut self.types,
            Namespace::Value => &mut self.values,
        };
        names
            .entry(name)
            .and_modify(|old| *old = Binding::Other)
            .or_insert(binding);
    }
}

/// One module's direct items, separate from the names visible in other modules or blocks.
#[derive(Debug, Default)]
pub(crate) struct Module<'t> {
    pub body: Range<usize>,
    pub parent: usize,
    /// Unconditional child modules: an inline scope index, or `None` for a file declaration.
    pub children: BTreeMap<&'t str, Option<usize>>,
    /// Unconditional concrete types, not aliases that can stand for API types.
    pub types: BTreeSet<&'t str>,
    /// The subset that also declare a tuple constructor in the value namespace.
    pub tuple_constructors: BTreeSet<&'t str>,
    /// The variants of each unconditional enum. A duplicate or conditional variant is unknown.
    pub variants: BTreeMap<&'t str, BTreeMap<&'t str, Variant>>,
    /// Names directly in this module, not in its blocks or child modules.
    pub names: ScopeNames<'t>,
    /// A block item or type parameter may shadow the first segment of a relative path.
    pub block_items: bool,
    /// A block value item may shadow an unqualified constructor.
    pub block_values: bool,
    pub type_params: BTreeSet<&'t str>,
}

/// What the parser reads of one file.
#[derive(Debug, Default)]
pub(crate) struct Syntax<'t> {
    pub builds: Vec<Build<'t>>,
    pub tuple_fields: Vec<TupleField<'t>>,
    /// Every path written in code, attributes included, `use` trees left out: its names.
    pub paths: Vec<Vec<&'t str>>,
    pub impls: Vec<Impl<'t>>,
    /// The structs, enums, unions and type aliases the file defines at its top level, under no
    /// `#[cfg]` or `#[cfg_attr]`.
    pub defines: BTreeSet<&'t str>,
    /// Every struct, enum and union defined anywhere, and every enum variant.
    pub types: BTreeSet<&'t str>,
    /// Every trait defined anywhere.
    pub traits: BTreeSet<&'t str>,
    /// `type X = …Y<…>;` anywhere: (X, Y), and whether it is an item of its own (not an
    /// associated type of an impl or trait).
    pub type_aliases: Vec<(&'t str, &'t str, bool)>,
    /// The file holds an inline `mod name { … }`, so it is more than one scope.
    pub inline_modules: bool,
    /// Every module the file declares, inline or not (`mod api;`).
    pub modules: BTreeSet<&'t str>,
    /// Names from `extern crate dep as name` that do not refer to the API itself.
    pub extern_aliases: BTreeSet<&'t str>,
    /// The first bytes of unconditional `use` statements directly in a module.
    pub direct_uses: BTreeSet<usize>,
    /// The file's top-level items and its inline modules, and their indices in source order.
    pub scopes: Vec<Module<'t>>,
    scope_order: Vec<usize>,
    /// The bytes inside each macro call's delimiters, and each `macro_rules!` body.
    pub macro_bodies: Vec<Range<usize>>,
}

impl Syntax<'_> {
    pub fn module_at(&self, byte: usize) -> usize {
        let after = self
            .scope_order
            .partition_point(|&scope| self.scopes[scope].body.start <= byte);
        let mut scope = self.scope_order[after - 1];
        while byte >= self.scopes[scope].body.end && scope != 0 {
            scope = self.scopes[scope].parent;
        }
        scope
    }
}

/// The path a type is, through parentheses and invisible groups; `None` for any other type.
fn plain_path(ty: &syn::Type) -> Option<&syn::Path> {
    match ty {
        syn::Type::Path(t) if t.qself.is_none() => Some(&t.path),
        syn::Type::Paren(t) => plain_path(&t.elem),
        syn::Type::Group(t) => plain_path(&t.elem),
        _ => None,
    }
}

/// Whether an item carries `#[cfg(…)]` or `#[cfg_attr(…)]`.
fn conditional(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
}

/// The path a type is through `&`, `&mut`, parentheses and invisible groups, with whether it
/// is written with a leading `::`; `None` for any other type.
fn type_path<'t>(reader: &Reader<'t>, ty: &syn::Type) -> Option<Tie<'t>> {
    match ty {
        syn::Type::Reference(r) => type_path(reader, &r.elem),
        syn::Type::Paren(t) => type_path(reader, &t.elem),
        syn::Type::Group(t) => type_path(reader, &t.elem),
        _ => plain_path(ty).map(|path| reader.path_tie(path, Namespace::Type)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Namespace {
    Type,
    /// A call or tuple pattern resolves a constructor, not a type name.
    Value,
}

/// A path that may tie a positional read to a struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tie<'t> {
    pub path: Vec<&'t str>,
    /// Written with a leading `::`.
    pub rooted: bool,
    pub namespace: Namespace,
}

/// Value bindings in declaration order, with each name's active ties (innermost last).
#[derive(Default)]
struct Locals<'t> {
    order: Vec<&'t str>,
    ties: BTreeMap<&'t str, Vec<Option<Tie<'t>>>>,
}

impl<'t> Locals<'t> {
    fn len(&self) -> usize {
        self.order.len()
    }

    fn push(&mut self, name: &'t str, tie: Option<Tie<'t>>) {
        self.order.push(name);
        self.ties.entry(name).or_default().push(tie);
    }

    fn truncate(&mut self, len: usize) {
        for name in self.order.drain(len..) {
            if let std::collections::btree_map::Entry::Occupied(mut entry) = self.ties.entry(name) {
                entry.get_mut().pop();
                if entry.get().is_empty() {
                    entry.remove();
                }
            }
        }
    }
}

struct Reader<'t> {
    text: &'t str,
    out: Syntax<'t>,
    module: usize,
    bindings: Locals<'t>,
    blocks: Vec<ScopeNames<'t>>,
    generics: Vec<(Namespace, &'t str)>,
    in_impl: bool,
}

impl<'t> Reader<'t> {
    /// An identifier's name as written in the text, `r#` left out.
    fn name(&self, ident: &syn::Ident) -> &'t str {
        let written = self.text.get(ident.span().byte_range()).unwrap_or("");
        written.strip_prefix("r#").unwrap_or(written)
    }

    fn names(&self, path: &syn::Path) -> Vec<&'t str> {
        path.segments.iter().map(|s| self.name(&s.ident)).collect()
    }

    fn path_tie(&self, path: &syn::Path, namespace: Namespace) -> Tie<'t> {
        Tie {
            path: self.names(path),
            rooted: path.leading_colon.is_some(),
            namespace,
        }
    }

    /// Collect item names before visiting bodies: block and module items are in scope
    /// throughout their block or module, even before their declaration.
    fn item_names(&self, names: &mut ScopeNames<'t>, item: &syn::Item, file_top: bool) {
        let (ident, space, binding) = match item {
            syn::Item::Enum(i) if !conditional(&i.attrs) => {
                (&i.ident, Namespace::Type, Binding::Enum)
            }
            syn::Item::Enum(i) => (&i.ident, Namespace::Type, Binding::Other),
            syn::Item::Struct(i) => {
                if !matches!(i.fields, syn::Fields::Named(_)) {
                    names.insert(Namespace::Value, self.name(&i.ident), Binding::Other);
                }
                (&i.ident, Namespace::Type, Binding::Other)
            }
            syn::Item::Union(i) => (&i.ident, Namespace::Type, Binding::Other),
            syn::Item::Mod(i) if !conditional(&i.attrs) => {
                (&i.ident, Namespace::Type, Binding::Module)
            }
            syn::Item::Mod(i) => (&i.ident, Namespace::Type, Binding::Other),
            syn::Item::Trait(i) => (&i.ident, Namespace::Type, Binding::Other),
            syn::Item::TraitAlias(i) => (&i.ident, Namespace::Type, Binding::Other),
            syn::Item::Type(i) => (&i.ident, Namespace::Type, Binding::Other),
            syn::Item::ExternCrate(i) => (
                i.rename.as_ref().map_or(&i.ident, |(_, name)| name),
                Namespace::Type,
                Binding::Other,
            ),
            syn::Item::Const(i) => (&i.ident, Namespace::Value, Binding::Other),
            syn::Item::Static(i) => (&i.ident, Namespace::Value, Binding::Other),
            syn::Item::Fn(i) => (&i.sig.ident, Namespace::Value, Binding::Other),
            syn::Item::ForeignMod(i) => {
                for item in &i.items {
                    let (ident, space) = match item {
                        syn::ForeignItem::Fn(i) => (&i.sig.ident, Namespace::Value),
                        syn::ForeignItem::Static(i) => (&i.ident, Namespace::Value),
                        syn::ForeignItem::Type(i) => (&i.ident, Namespace::Type),
                        _ => continue,
                    };
                    names.insert(space, self.name(ident), Binding::Other);
                }
                return;
            }
            syn::Item::Use(u) => {
                // Match the original tree, not flattened paths: a group is not proof.
                let exact = file_top && u.leading_colon.is_none() && !conditional(&u.attrs);
                let binding = if exact
                    && let syn::UseTree::Path(root) = &u.tree
                    && self.name(&root.ident) == "self"
                    && let syn::UseTree::Path(owner) = &*root.tree
                {
                    match &*owner.tree {
                        syn::UseTree::Name(v) => Binding::VariantImport {
                            owner: self.name(&owner.ident),
                            variant: self.name(&v.ident),
                        },
                        syn::UseTree::Rename(v) => Binding::VariantImport {
                            owner: self.name(&owner.ident),
                            variant: self.name(&v.ident),
                        },
                        _ => Binding::Other,
                    }
                } else {
                    Binding::Other
                };
                self.use_names(names, &u.tree, None, binding);
                return;
            }
            _ => return,
        };
        names.insert(space, self.name(ident), binding);
    }

    fn use_names(
        &self,
        names: &mut ScopeNames<'t>,
        tree: &syn::UseTree,
        prefix: Option<&'t str>,
        binding: Binding<'t>,
    ) {
        let local = match tree {
            syn::UseTree::Path(path) => {
                self.use_names(names, &path.tree, Some(self.name(&path.ident)), binding);
                return;
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.use_names(names, tree, prefix, binding);
                }
                return;
            }
            syn::UseTree::Glob(_) => {
                names.glob = true;
                return;
            }
            syn::UseTree::Name(name) => match self.name(&name.ident) {
                "self" => prefix.unwrap_or("self"),
                name => name,
            },
            syn::UseTree::Rename(rename) => self.name(&rename.rename),
        };
        if local != "_" {
            // The import's namespace is not known until its target is resolved.
            names.insert(Namespace::Type, local, binding);
            names.insert(Namespace::Value, local, binding);
        }
    }

    fn module_items(&mut self, items: &[syn::Item]) {
        let mut names = ScopeNames::default();
        for item in items {
            self.item_names(&mut names, item, self.module == 0);
        }
        self.out.scopes[self.module].names = names;
        for item in items {
            let concrete = match item {
                syn::Item::Struct(i) if !conditional(&i.attrs) => {
                    if matches!(i.fields, syn::Fields::Unnamed(_)) {
                        let name = self.name(&i.ident);
                        self.out.scopes[self.module].tuple_constructors.insert(name);
                    }
                    Some(&i.ident)
                }
                syn::Item::Enum(i) if !conditional(&i.attrs) => {
                    let name = self.name(&i.ident);
                    let mut variants = BTreeMap::new();
                    for v in &i.variants {
                        let shape = (!conditional(&v.attrs)).then_some(match v.fields {
                            syn::Fields::Named(_) => VariantShape::Named,
                            syn::Fields::Unnamed(_) => VariantShape::Tuple,
                            syn::Fields::Unit => VariantShape::Unit,
                        });
                        let name = self.name(&v.ident);
                        let has_tuple = shape == Some(VariantShape::Tuple);
                        variants
                            .entry(name)
                            .and_modify(|old: &mut Variant| {
                                old.shape = None;
                                old.has_tuple |= has_tuple;
                            })
                            .or_insert(Variant { shape, has_tuple });
                    }
                    self.out.scopes[self.module].variants.insert(name, variants);
                    Some(&i.ident)
                }
                syn::Item::Union(i) if !conditional(&i.attrs) => Some(&i.ident),
                _ => None,
            };
            if let Some(ident) = concrete {
                let name = self.name(ident);
                self.out.scopes[self.module].types.insert(name);
                if self.module == 0 {
                    self.out.defines.insert(name);
                }
            }
            // For the existing unqualified-name check, a readable alias is also a definition.
            if self.module == 0
                && let syn::Item::Type(i) = item
                && !conditional(&i.attrs)
                && plain_path(&i.ty).is_some()
            {
                self.out.defines.insert(self.name(&i.ident));
            }
            match item {
                syn::Item::Mod(m) => self.module_item(m, true),
                syn::Item::Use(u) if !conditional(&u.attrs) => {
                    self.out
                        .direct_uses
                        .insert(u.use_token.span.byte_range().start);
                    self.visit_item_use(u);
                }
                item => self.visit_item(item),
            }
        }
    }

    fn module_item(&mut self, m: &syn::ItemMod, direct: bool) {
        let name = self.name(&m.ident);
        self.out.modules.insert(name);
        let parent = self.module;
        if direct && !conditional(&m.attrs) && !m.attrs.iter().any(|a| a.path().is_ident("path")) {
            let inline = m.content.as_ref().map(|_| self.out.scopes.len());
            self.out.scopes[parent].children.insert(name, inline);
        }
        if let Some((brace, items)) = &m.content {
            self.out.inline_modules = true;
            self.module = self.out.scopes.len();
            self.out.scopes.push(Module {
                body: brace.span.join().byte_range(),
                parent,
                ..Module::default()
            });
            let blocks = std::mem::take(&mut self.blocks);
            let generics = std::mem::take(&mut self.generics);
            let bindings = std::mem::take(&mut self.bindings);
            for attr in &m.attrs {
                self.visit_attribute(attr);
            }
            self.module_items(items);
            self.bindings = bindings;
            self.generics = generics;
            self.blocks = blocks;
            self.module = parent;
        } else {
            visit::visit_item_mod(self, m);
        }
    }

    fn alias(&mut self, alias: &syn::Ident, ty: &syn::Type, item: bool) {
        // An alias of a type the scan cannot read is not recorded, so a literal of its name is
        // of a name the scan cannot place.
        let alias = self.name(alias);
        if let Some(last) = plain_path(ty).and_then(|path| path.segments.last()) {
            let target = self.name(&last.ident);
            self.out.type_aliases.push((alias, target, item));
        }
    }

    /// Binds every name `pat` binds, tied to no struct.
    fn bind_untied(&mut self, pat: &syn::Pat) {
        struct Names<'r, 't>(&'r mut Reader<'t>);
        impl<'a> Visit<'a> for Names<'_, '_> {
            fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
                let name = self.0.name(&p.ident);
                self.0.bindings.push(name, None);
                visit::visit_pat_ident(self, p);
            }
        }
        Names(self).visit_pat(pat);
    }

    /// Binds a parameter or a `let` pattern: a name with a type is tied to the type's path.
    fn bind(&mut self, pat: &syn::Pat, tie: Option<Tie<'t>>) {
        match (pat, tie) {
            (syn::Pat::Type(typed), _) => {
                let tie = type_path(self, &typed.ty);
                self.bind(&typed.pat, tie);
            }
            (syn::Pat::Ident(p), tie) if p.subpat.is_none() => {
                let name = self.name(&p.ident);
                self.bindings.push(name, tie);
            }
            (pat, _) => self.bind_untied(pat),
        }
    }

    /// The path a value is tied to: a possible constructor call, a name bound with its type,
    /// or `self` in a method. Anything else (a tuple, a field, a method's result, a closure's
    /// untyped argument) is tied to none.
    fn tie(&self, expr: &syn::Expr) -> Option<Tie<'t>> {
        match expr {
            syn::Expr::Paren(e) => self.tie(&e.expr),
            syn::Expr::Group(e) => self.tie(&e.expr),
            syn::Expr::Call(call) => match &*call.func {
                syn::Expr::Path(p) if p.qself.is_none() => {
                    Some(self.path_tie(&p.path, Namespace::Value))
                }
                _ => None,
            },
            syn::Expr::Path(p) if p.qself.is_none() => {
                let ident = p.path.get_ident()?;
                let name = self.name(ident);
                self.bindings.ties.get(name)?.last()?.clone()
            }
            _ => None,
        }
    }

    fn tuple_field(&mut self, tie: Tie<'t>, position: u32, at: proc_macro2::Span) {
        self.out.tuple_fields.push(TupleField {
            tie,
            position,
            line: at.start().line,
            byte: at.byte_range().start,
        });
    }

    /// Reads a signature before binding its parameters, then reads the body.
    fn function(
        &mut self,
        attrs: &[syn::Attribute],
        vis: Option<&syn::Visibility>,
        sig: &syn::Signature,
        body: Option<&syn::Block>,
    ) {
        let outer = std::mem::take(&mut self.bindings);
        self.with_generics(&sig.generics, |this| {
            for attr in attrs {
                this.visit_attribute(attr);
            }
            if let Some(vis) = vis {
                this.visit_visibility(vis);
            }
            this.visit_signature(sig);
            if let Some(body) = body {
                for input in &sig.inputs {
                    match input {
                        syn::FnArg::Receiver(r) => {
                            let tie = match r.colon_token {
                                None => Some(Tie {
                                    path: vec!["Self"],
                                    rooted: false,
                                    namespace: Namespace::Type,
                                }),
                                Some(_) => type_path(this, &r.ty),
                            };
                            this.bindings.push("self", tie);
                        }
                        syn::FnArg::Typed(t) => this.bind(&t.pat, None),
                    }
                }
                this.visit_block(body);
            }
        });
        self.bindings = outer;
    }

    fn with_generics(&mut self, generics: &syn::Generics, read: impl FnOnce(&mut Self)) {
        let outer = self.generics.len();
        for param in &generics.params {
            let (space, name) = match param {
                syn::GenericParam::Type(p) => (Namespace::Type, self.name(&p.ident)),
                syn::GenericParam::Const(p) => (Namespace::Value, self.name(&p.ident)),
                syn::GenericParam::Lifetime(_) => continue,
            };
            self.generics.push((space, name));
        }
        read(self);
        self.generics.truncate(outer);
    }

    fn shadowed(&self, namespace: Namespace, name: &str) -> bool {
        self.generics.contains(&(namespace, name))
            || self
                .blocks
                .iter()
                .rev()
                .any(|scope| scope.glob || scope.get(namespace, name).is_some())
            || (namespace == Namespace::Value && self.bindings.ties.contains_key(name))
    }

    fn build(
        &mut self,
        path: &syn::Path,
        how: How,
        brace: bool,
        variant: VariantUse,
        qself: Option<&syn::QSelf>,
    ) {
        let span = match (&path.leading_colon, path.segments.first()) {
            (Some(colons), _) => colons.spans[0],
            (None, Some(first)) => first.ident.span(),
            (None, None) => return,
        };
        let names = self.names(path);
        let namespace = if names.len() > 1 || variant == VariantUse::Braced {
            Namespace::Type
        } else {
            Namespace::Value
        };
        let clear = qself.is_none()
            && match names.as_slice() {
                _ if path.leading_colon.is_some() => true,
                ["Self", ..] => self.in_impl,
                ["self" | "super" | "crate", ..] => true,
                [first, ..] => !self.shadowed(namespace, first),
                [] => false,
            };
        self.out.builds.push(Build {
            path: names,
            rooted: path.leading_colon.is_some(),
            how,
            brace,
            line: span.start().line,
            byte: span.byte_range().start,
            variant: clear.then_some(variant),
        });
    }
}

impl<'a, 't> Visit<'a> for Reader<'t> {
    fn visit_item(&mut self, item: &'a syn::Item) {
        // A nested item does not inherit the containing impl's `Self`.
        let in_impl = std::mem::replace(&mut self.in_impl, false);
        let generics = match item {
            syn::Item::Const(i) => Some(&i.generics),
            syn::Item::Enum(i) => Some(&i.generics),
            syn::Item::Struct(i) => Some(&i.generics),
            syn::Item::Trait(i) => Some(&i.generics),
            syn::Item::TraitAlias(i) => Some(&i.generics),
            syn::Item::Type(i) => Some(&i.generics),
            syn::Item::Union(i) => Some(&i.generics),
            _ => None,
        };
        if let Some(generics) = generics {
            self.with_generics(generics, |this| visit::visit_item(this, item));
        } else {
            visit::visit_item(self, item);
        }
        self.in_impl = in_impl;
    }
    fn visit_expr_struct(&mut self, e: &'a syn::ExprStruct) {
        let how = if e.dot2_token.is_some() || e.rest.is_some() {
            How::Base
        } else {
            How::Full
        };
        self.build(&e.path, how, true, VariantUse::Braced, e.qself.as_ref());
        visit::visit_expr_struct(self, e);
    }

    fn visit_expr_call(&mut self, e: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*e.func {
            self.build(
                &p.path,
                How::Full,
                false,
                VariantUse::Tuple,
                p.qself.as_ref(),
            );
        }
        visit::visit_expr_call(self, e);
    }

    fn visit_expr_path(&mut self, e: &'a syn::ExprPath) {
        // A value (and in a pattern, `Pat::Path`): a tuple struct's constructor, a unit struct.
        self.build(
            &e.path,
            How::Value,
            false,
            VariantUse::Value,
            e.qself.as_ref(),
        );
        visit::visit_expr_path(self, e);
    }

    fn visit_pat_struct(&mut self, p: &'a syn::PatStruct) {
        // With `..` a pattern reads the fields it names; without, it lists them all.
        if p.rest.is_none() {
            self.build(
                &p.path,
                How::Full,
                false,
                VariantUse::Braced,
                p.qself.as_ref(),
            );
        }
        visit::visit_pat_struct(self, p);
    }

    fn visit_pat_tuple_struct(&mut self, p: &'a syn::PatTupleStruct) {
        if !p.elems.iter().any(|e| matches!(e, syn::Pat::Rest(_))) {
            self.build(
                &p.path,
                How::Full,
                false,
                VariantUse::Tuple,
                p.qself.as_ref(),
            );
        }
        // The positions before a `..` are known; those after it count from an end the scan
        // does not know.
        if let Some(first) = p.path.segments.first() {
            let tie = self.path_tie(&p.path, Namespace::Value);
            let at = first.ident.span();
            let listed = p
                .elems
                .iter()
                .take_while(|e| !matches!(e, syn::Pat::Rest(_)));
            for (position, _) in (0..).zip(listed) {
                self.tuple_field(tie.clone(), position, at);
            }
        }
        visit::visit_pat_tuple_struct(self, p);
    }

    fn visit_expr_field(&mut self, e: &'a syn::ExprField) {
        if let syn::Member::Unnamed(index) = &e.member
            && let Some(tie) = self.tie(&e.base)
        {
            self.tuple_field(tie, index.index, index.span);
        }
        visit::visit_expr_field(self, e);
    }

    fn visit_item_fn(&mut self, i: &'a syn::ItemFn) {
        self.function(&i.attrs, Some(&i.vis), &i.sig, Some(&i.block));
    }

    fn visit_impl_item_fn(&mut self, i: &'a syn::ImplItemFn) {
        self.function(&i.attrs, Some(&i.vis), &i.sig, Some(&i.block));
    }

    fn visit_trait_item_fn(&mut self, i: &'a syn::TraitItemFn) {
        self.function(&i.attrs, None, &i.sig, i.default.as_ref());
    }

    fn visit_type_param(&mut self, p: &'a syn::TypeParam) {
        let name = self.name(&p.ident);
        self.out.scopes[self.module].type_params.insert(name);
        visit::visit_type_param(self, p);
    }

    fn visit_block(&mut self, b: &'a syn::Block) {
        let outer = self.bindings.len();
        let mut names = ScopeNames::default();
        for stmt in &b.stmts {
            if let syn::Stmt::Item(item) = stmt {
                self.item_names(&mut names, item, false);
                let module = &mut self.out.scopes[self.module];
                match item {
                    syn::Item::Mod(_)
                    | syn::Item::ExternCrate(_)
                    | syn::Item::Struct(_)
                    | syn::Item::Enum(_)
                    | syn::Item::Union(_)
                    | syn::Item::Trait(_)
                    | syn::Item::TraitAlias(_)
                    | syn::Item::Type(_) => module.block_items = true,
                    syn::Item::Const(_)
                    | syn::Item::Fn(_)
                    | syn::Item::ForeignMod(_)
                    | syn::Item::Static(_) => {
                        module.block_values = true;
                    }
                    _ => {}
                }
            }
        }
        self.blocks.push(names);
        visit::visit_block(self, b);
        self.blocks.pop();
        self.bindings.truncate(outer);
    }

    fn visit_local(&mut self, l: &'a syn::Local) {
        // The value is read before the pattern binds: `let v = v.0;`.
        for attr in &l.attrs {
            self.visit_attribute(attr);
        }
        if let Some(init) = &l.init {
            self.visit_local_init(init);
        }
        self.visit_pat(&l.pat);
        let tie = l.init.as_ref().and_then(|init| self.tie(&init.expr));
        self.bind(&l.pat, tie);
    }

    fn visit_expr_closure(&mut self, e: &'a syn::ExprClosure) {
        let outer = self.bindings.len();
        for attr in &e.attrs {
            self.visit_attribute(attr);
        }
        if let Some(lifetimes) = &e.lifetimes {
            self.visit_bound_lifetimes(lifetimes);
        }
        for input in &e.inputs {
            self.visit_pat(input);
        }
        self.visit_return_type(&e.output);
        for input in &e.inputs {
            self.bind(input, None);
        }
        self.visit_expr(&e.body);
        self.bindings.truncate(outer);
    }

    fn visit_arm(&mut self, a: &'a syn::Arm) {
        let outer = self.bindings.len();
        for attr in &a.attrs {
            self.visit_attribute(attr);
        }
        self.visit_pat(&a.pat);
        self.bind_untied(&a.pat);
        if let Some((_, guard)) = &a.guard {
            self.visit_expr(guard);
        }
        self.visit_expr(&a.body);
        self.bindings.truncate(outer);
    }

    fn visit_expr_for_loop(&mut self, e: &'a syn::ExprForLoop) {
        let outer = self.bindings.len();
        for attr in &e.attrs {
            self.visit_attribute(attr);
        }
        if let Some(label) = &e.label {
            self.visit_label(label);
        }
        self.visit_pat(&e.pat);
        self.visit_expr(&e.expr);
        self.bind_untied(&e.pat);
        self.visit_block(&e.body);
        self.bindings.truncate(outer);
    }

    fn visit_expr_let(&mut self, e: &'a syn::ExprLet) {
        visit::visit_expr_let(self, e);
        self.bind_untied(&e.pat);
    }

    fn visit_expr_if(&mut self, e: &'a syn::ExprIf) {
        // What `if let` binds holds in its first branch only.
        let outer = self.bindings.len();
        for attr in &e.attrs {
            self.visit_attribute(attr);
        }
        self.visit_expr(&e.cond);
        self.visit_block(&e.then_branch);
        self.bindings.truncate(outer);
        if let Some((_, otherwise)) = &e.else_branch {
            self.visit_expr(otherwise);
        }
    }

    fn visit_expr_while(&mut self, e: &'a syn::ExprWhile) {
        let outer = self.bindings.len();
        visit::visit_expr_while(self, e);
        self.bindings.truncate(outer);
    }

    fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
        // `Marker` alone in a pattern parses as a binding; it may be a unit struct.
        if p.subpat.is_none() && p.by_ref.is_none() && p.mutability.is_none() {
            self.build(
                &syn::Path::from(p.ident.clone()),
                How::Value,
                false,
                VariantUse::Value,
                None,
            );
        }
        visit::visit_pat_ident(self, p);
    }

    fn visit_path(&mut self, p: &'a syn::Path) {
        let names = self.names(p);
        self.out.paths.push(names);
        visit::visit_path(self, p);
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        self.with_generics(&i.generics, |this| {
            let target = plain_path(&i.self_ty).map(|path| {
                let mut target = this.names(path);
                if path.leading_colon.is_some() {
                    target.insert(0, "");
                }
                target
            });
            let shadowed_target = target.as_ref().is_some_and(|target| match &target[..] {
                ["self" | "super" | "crate", ..] => false,
                [first, ..] => this.shadowed(Namespace::Type, first),
                [] => false,
            });
            this.out.impls.push(Impl {
                body: i.brace_token.span.join().byte_range(),
                target,
                shadowed_target,
            });
            this.in_impl = true;
            visit::visit_item_impl(this, i);
        });
    }

    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        self.module_item(m, false);
    }

    fn visit_item_extern_crate(&mut self, item: &'a syn::ItemExternCrate) {
        if self.name(&item.ident) != "mod_api"
            && let Some((_, name)) = &item.rename
        {
            let name = self.name(name);
            self.out.extern_aliases.insert(name);
        }
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_item_struct(&mut self, i: &'a syn::ItemStruct) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'a syn::ItemEnum) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        for variant in &i.variants {
            let name = self.name(&variant.ident);
            self.out.types.insert(name);
        }
        visit::visit_item_enum(self, i);
    }

    fn visit_item_union(&mut self, i: &'a syn::ItemUnion) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        visit::visit_item_union(self, i);
    }

    fn visit_item_trait(&mut self, i: &'a syn::ItemTrait) {
        let name = self.name(&i.ident);
        self.out.traits.insert(name);
        visit::visit_item_trait(self, i);
    }

    fn visit_item_type(&mut self, i: &'a syn::ItemType) {
        self.alias(&i.ident, &i.ty, true);
        visit::visit_item_type(self, i);
    }

    fn visit_impl_item_type(&mut self, i: &'a syn::ImplItemType) {
        self.alias(&i.ident, &i.ty, false);
        self.with_generics(&i.generics, |this| visit::visit_impl_item_type(this, i));
    }

    fn visit_impl_item_const(&mut self, i: &'a syn::ImplItemConst) {
        self.with_generics(&i.generics, |this| visit::visit_impl_item_const(this, i));
    }

    fn visit_trait_item_const(&mut self, i: &'a syn::TraitItemConst) {
        self.with_generics(&i.generics, |this| visit::visit_trait_item_const(this, i));
    }

    fn visit_trait_item_type(&mut self, i: &'a syn::TraitItemType) {
        self.with_generics(&i.generics, |this| visit::visit_trait_item_type(this, i));
    }

    fn visit_macro(&mut self, m: &'a syn::Macro) {
        let span = match &m.delimiter {
            syn::MacroDelimiter::Paren(d) => d.span.join(),
            syn::MacroDelimiter::Brace(d) => d.span.join(),
            syn::MacroDelimiter::Bracket(d) => d.span.join(),
        };
        self.out.macro_bodies.push(span.byte_range());
        visit::visit_macro(self, m);
    }
}

/// The deepest the parser is let recurse, counted by [`too_deep`].
pub(crate) const MAX_NESTING: usize = 256;

/// Keywords after which a token starts an operand rather than continuing one.
pub(crate) const KEYWORDS: &[&str] = &[
    "as", "async", "box", "break", "const", "continue", "dyn", "else", "enum", "extern", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "trait", "type", "unsafe", "use", "where", "while", "yield",
];

/// Keywords the parser reads what follows by recursion: `return return x`, `fn() -> fn()`,
/// `if … else if …`.
const RECURSIVE: &[&str] = &[
    "async", "box", "break", "dyn", "fn", "if", "impl", "let", "move", "ref", "return", "yield",
];

/// What the nesting count is inside: a bracket, a generic `<`, or a closure's parameters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Bracket,
    Angle,
    /// A `<` straight after a literal, `)`, `]` or `?`, which compares or shifts, unless a type
    /// follows it: counted like a generic, but ended by a `,` like an expression.
    Compare,
    Params,
}

/// The first line where the parser would recurse deeper than [`MAX_NESTING`].
///
/// The count is taken along the path of nesting. Each open bracket, generic `<` and closure
/// counts until it closes, and so does each assignment, `@` and keyword the parser reads what
/// follows by recursion. A run of prefix operators counts until its operand ends, through
/// method calls and fields (`-x.f(…)`), and no further than the bracket after it. A count is
/// dropped only where the parser has certainly returned: at the bracket that encloses it
/// closing, at a `;`, at a match arm's `=>`, at a `}` that ends a statement, and at a `,`
/// directly inside a bracket (between elements, arms or fields). A `,` inside a generic or a
/// closure's parameters drops nothing, since generic arguments and parameters are listed with
/// commas. A `<` straight after a literal, `)`, `]` or `?` counts like any other, but a `,`
/// ends it, as it ends the comparison or shift it most likely is (`[1 << 0, 1 << 1]`).
///
/// This is a cheap filter, not the guarantee: the scan's parse runs in a child process first
/// ([`crate::child`]), so a parse that still runs out of stack ends the child, not the caller.
pub(crate) fn too_deep(tokens: &[(Token<'_>, usize)]) -> Option<usize> {
    let token = |at: usize| tokens.get(at).map(|(t, _)| *t);
    let punct = |at: usize, byte: u8| token(at) == Some(Token::Punct(byte));
    // Whether the token before `at` ends an operand, so an operator at `at` is binary.
    let operand_end = |at: usize| match at.checked_sub(1).and_then(token) {
        Some(Token::Literal(_) | Token::Punct(b')' | b']' | b'}' | b'?')) => true,
        Some(Token::Ident(name)) => !KEYWORDS.contains(&name),
        _ => false,
    };
    let mut frames: Vec<(usize, Frame)> = Vec::new();
    // The count that stays until a drop, and the prefix operators of the current operand.
    let (mut depth, mut run) = (0usize, 0usize);
    let mut skip = false;
    // Drops the generics and closure parameters above the innermost bracket, and the count of
    // the statement inside it.
    let end_statement = |frames: &mut Vec<(usize, Frame)>| {
        while frames
            .last()
            .is_some_and(|(_, frame)| *frame != Frame::Bracket)
        {
            frames.pop();
        }
        frames.last().map_or(0, |(before, _)| before + 1)
    };
    for (at, (current, line)) in tokens.iter().enumerate() {
        if skip {
            skip = false;
            continue;
        }
        match current {
            // A prefix operator's count ends where the bracket after it closes.
            Token::Punct(b'(' | b'[' | b'{') => {
                frames.push((depth, Frame::Bracket));
                depth += run + 1;
                run = 0;
            }
            Token::Punct(b')' | b']' | b'}') => {
                while let Some((before, frame)) = frames.pop() {
                    depth = before;
                    if frame == Frame::Bracket {
                        break;
                    }
                }
                run = 0;
                // A block that ends its statement: the next one starts afresh.
                if punct(at, b'}')
                    && match token(at + 1) {
                        None | Some(Token::Punct(b'}' | b';' | b',' | b'#' | b')' | b']')) => true,
                        Some(Token::Ident(next)) => !matches!(next, "else" | "as" | "in"),
                        _ => false,
                    }
                {
                    depth = end_statement(&mut frames);
                }
            }
            // A match arm's pattern and guard end at `=>`, generics and comparisons with them.
            Token::Punct(b';') | Token::Punct(b'=') if punct(at, b';') || punct(at + 1, b'>') => {
                depth = end_statement(&mut frames);
                run = 0;
            }
            Token::Punct(b',') => {
                while let Some((before, Frame::Compare)) = frames.last().copied() {
                    frames.pop();
                    depth = before;
                }
                if frames
                    .last()
                    .is_none_or(|(_, frame)| *frame == Frame::Bracket)
                {
                    depth = end_statement(&mut frames);
                }
                run = 0;
            }
            Token::Punct(b'<') if punct(at + 1, b'=') => run = 0,
            Token::Punct(b'<') => {
                // The second `<` of a shift (`1 << 2`) compares too.
                let compare = match at.checked_sub(1).and_then(token) {
                    Some(Token::Literal(_) | Token::Punct(b')' | b']' | b'?')) => true,
                    Some(Token::Punct(b'<')) => {
                        matches!(frames.last(), Some((_, Frame::Compare)))
                    }
                    _ => false,
                };
                let frame = if compare {
                    Frame::Compare
                } else {
                    Frame::Angle
                };
                frames.push((depth + run, frame));
                depth += run + 1;
                run = 0;
            }
            Token::Punct(b'>') => {
                // Not `->`, `=>` or `>=`: the close of the innermost generic, if one is open.
                if !(at >= 1 && (punct(at - 1, b'-') || punct(at - 1, b'=')))
                    && !punct(at + 1, b'=')
                    && let Some((before, Frame::Angle | Frame::Compare)) = frames.last().copied()
                {
                    frames.pop();
                    depth = before;
                }
                run = 0;
            }
            Token::Punct(b'|') => {
                if frames
                    .last()
                    .is_some_and(|(_, frame)| *frame == Frame::Params)
                {
                    // The parameters end; the closure's body follows.
                    if let Some((before, _)) = frames.pop() {
                        depth = before + 1;
                    }
                } else if operand_end(at) {
                    // A binary `|` or `||`.
                    skip = punct(at + 1, b'|');
                } else if punct(at + 1, b'|') {
                    // A closure with no parameters.
                    depth += run + 1;
                    skip = true;
                } else {
                    frames.push((depth + run, Frame::Params));
                    depth += run + 1;
                }
                run = 0;
            }
            Token::Punct(b'=')
                if !punct(at + 1, b'=')
                    && !punct(at + 1, b'>')
                    && !(at >= 1
                        && matches!(
                            token(at - 1),
                            Some(Token::Punct(b'=' | b'!' | b'<' | b'>'))
                        )) =>
            {
                depth += 1;
                run = 0;
            }
            Token::Punct(b'@') => {
                depth += 1;
                run = 0;
            }
            Token::Punct(b'-' | b'!' | b'*' | b'&' | b'.') if !operand_end(at) => run += 1,
            // A path's `::`, and a method call's or field's `.`, continue the operand.
            Token::Punct(b':' | b'.') => {}
            Token::Punct(_) => run = 0,
            Token::Ident(name) if RECURSIVE.contains(name) => depth += 1,
            _ => {}
        }
        if depth + run > MAX_NESTING {
            return Some(*line);
        }
    }
    None
}

/// The stack the parser runs on. Measured on 20 shapes nested as deep as [`too_deep`] accepts
/// (brackets, generics with and without commas, closures with one and two parameters,
/// references, `fn` types, assignments, `return`, `@`, `else if`, qualified paths after a
/// comparison, prefix operators on method calls, mixes of them), the deepest
/// needs 1.6 MiB in a release build and 1.5 MiB in a test build; this leaves a tenfold margin.
pub(crate) const PARSER_STACK: usize = 16 << 20;

/// Why a file was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// It does not parse, at this line.
    Parse(usize),
    /// The parser's own stack could not be reserved.
    NoStack,
}

/// Parses `text` and reads it, on a thread with a stack of its own.
pub(crate) fn read(text: &str) -> Result<Syntax<'_>, Unread> {
    // The parser reads nesting by recursion. [`too_deep`] bounds it, and a stack of its own
    // leaves room for any file within that bound, wherever the scan is called from. Without one
    // the file is not parsed at all.
    std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .stack_size(PARSER_STACK)
            .spawn_scoped(scope, || parse(text))
        {
            Ok(thread) => thread.join().unwrap_or(Err(Unread::NoStack)),
            Err(_) => Err(Unread::NoStack),
        }
    })
}

pub(crate) fn parse(text: &str) -> Result<Syntax<'_>, Unread> {
    // The parser skips a byte-order mark and a shebang line itself, which would shift every
    // byte it reports; blank them in a copy of the same length instead.
    let mut copy = text.to_owned();
    if copy.starts_with('\u{feff}') {
        copy.replace_range(..3, "   ");
    }
    if copy.starts_with("#!") && !copy.starts_with("#![") {
        let end = copy.find('\n').unwrap_or(copy.len());
        copy.replace_range(..end, &" ".repeat(end));
    }
    let parsed = syn::parse_str::<syn::File>(&copy);
    let result = match parsed {
        Ok(file) => {
            let mut reader = Reader {
                text,
                out: Syntax::default(),
                module: 0,
                bindings: Locals::default(),
                blocks: Vec::new(),
                generics: Vec::new(),
                in_impl: false,
            };
            reader.out.scopes.push(Module {
                body: 0..text.len(),
                ..Module::default()
            });
            for attr in &file.attrs {
                reader.visit_attribute(attr);
            }
            reader.module_items(&file.items);
            reader.out.scope_order = (0..reader.out.scopes.len()).collect();
            reader
                .out
                .scope_order
                .sort_by_key(|&scope| reader.out.scopes[scope].body.start);
            Ok(reader.out)
        }
        Err(error) => Err(Unread::Parse(error.span().start().line.max(1))),
    };
    // The parser keeps every text it read for its spans until told to forget them.
    proc_macro2::extra::invalidate_current_thread_spans();
    result
}
