//! Language table, parsing, declaration units and the semantic stream (spec
//! sections 6.1, 6.4, 6.5, 8.4, and `smallest_named_node` of section 16.1).
//!
//! The stream and unit functions are pure: no file, clock, environment or
//! process access. Every walk over a parse tree is iterative (section 6.1,
//! requirement R49): its stack use does not grow with the depth of the tree.
//! Offsets are UTF-8 bytes into the resource text of section 4.2.

use std::collections::{HashMap, HashSet};

use tree_sitter::{Language, Node, Parser, Tree};

use crate::fingerprint;
use crate::model::Fingerprint;
use crate::text;

// ---------------------------------------------------------------------------
// Resource class (section 6.1)
// ---------------------------------------------------------------------------

/// The resource class of a file, from the lower-cased extension of its path
/// (section 6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Markdown,
    Prose,
    Typescript,
    Tsx,
    Python,
    Rust,
    Go,
    Java,
    Text,
}

impl Class {
    /// Every class, in the order of the table of section 6.1.
    pub const ALL: [Class; 9] = [
        Class::Markdown,
        Class::Prose,
        Class::Typescript,
        Class::Tsx,
        Class::Python,
        Class::Rust,
        Class::Go,
        Class::Java,
        Class::Text,
    ];

    /// The class of a locator path. The extension is the part of the last
    /// path segment after its last `.`, lower-cased; a segment whose only
    /// `.` is its first character (`.md`) has no extension.
    pub fn from_path(path: &str) -> Class {
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
            _ => return Class::Text,
        };
        match ext.as_str() {
            "md" | "mdx" => Class::Markdown,
            "markdown" | "txt" | "rst" => Class::Prose,
            "ts" | "mts" | "cts" => Class::Typescript,
            "tsx" | "js" | "mjs" | "cjs" | "jsx" => Class::Tsx,
            "py" | "pyi" => Class::Python,
            "rs" => Class::Rust,
            "go" => Class::Go,
            "java" => Class::Java,
            _ => Class::Text,
        }
    }

    /// The class by its name of section 6.1.
    pub fn from_name(name: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The class name of section 6.1, also the `class` input of `ast-v1`.
    pub fn name(self) -> &'static str {
        match self {
            Class::Markdown => "markdown",
            Class::Prose => "prose",
            Class::Typescript => "typescript",
            Class::Tsx => "tsx",
            Class::Python => "python",
            Class::Rust => "rust",
            Class::Go => "go",
            Class::Java => "java",
            Class::Text => "text",
        }
    }

    /// True for the six classes with a grammar.
    pub fn is_code(self) -> bool {
        self.language().is_some()
    }

    /// The pinned grammar of a code class.
    pub fn language(self) -> Option<Language> {
        Some(match self {
            Class::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Class::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Class::Python => tree_sitter_python::LANGUAGE.into(),
            Class::Rust => tree_sitter_rust::LANGUAGE.into(),
            Class::Go => tree_sitter_go::LANGUAGE.into(),
            Class::Java => tree_sitter_java::LANGUAGE.into(),
            _ => return None,
        })
    }
}

/// Parse `text` with the grammar of `class`. `None` for a class without a
/// grammar. The tree may contain errors; see [`analyze`] for the fallback.
pub fn parse(text: &str, class: Class) -> Option<Tree> {
    let language = class.language()?;
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("the pinned grammars use an ABI that tree-sitter supports");
    parser.parse(text, None)
}

// ---------------------------------------------------------------------------
// Semantic stream (section 8.4)
// ---------------------------------------------------------------------------

/// Comment node kinds (section 8.4), plus `html_comment` of the TypeScript
/// and TSX grammars (ruling on section 8.4). A comment node is skipped with
/// everything under it, so Rust's `doc_comment`, which only occurs inside
/// `line_comment` and `block_comment`, needs no entry.
pub fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment" | "line_comment" | "block_comment" | "html_comment"
    )
}

/// The node kinds of a language whose only comma, before the closing
/// delimiter, makes a one-element tuple, so that removing it changes the
/// meaning (ruling on section 8.4): Rust `(u8,)` against `(u8)`, Python
/// `x[1,]` against `x[1]`. Only the Rust and Python grammars have such a
/// node; the TypeScript, Go and Java grammars have none. The language is
/// told by the grammar's name, which the pinned Rust and Python grammars
/// carry (ABI 15).
fn tuple_kinds(language: Option<&str>) -> &'static [&'static str] {
    match language {
        Some("rust") => &["tuple_expression", "tuple_type", "tuple_pattern"],
        Some("python") => &["tuple", "tuple_pattern", "subscript"],
        _ => &[],
    }
}

/// Content literal kinds (section 8.4).
fn is_content_literal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "string_literal"
            | "interpreted_string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "rune_literal"
            | "number"
            | "integer"
            | "float"
            | "integer_literal"
            | "float_literal"
            | "int_literal"
            | "imaginary_literal"
    )
}

/// True for a node the semantic stream skips with its subtree: a comment
/// node, or a TSX `jsx_expression` whose only content is comments, such as
/// `{/* note */}` or `{}` (ruling on section 8.4).
fn is_skipped(node: Node<'_>) -> bool {
    if is_comment_kind(node.kind()) {
        return true;
    }
    if node.kind() != "jsx_expression" {
        return false;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .all(|c| is_comment_kind(c.kind()))
}

fn node_text<'t>(node: Node<'_>, text: &'t str) -> &'t str {
    &text[node.byte_range()]
}

/// The `ast-v1` semantic stream of `node` (section 8.4): a pre-order walk
/// over all children, named and anonymous, in source order, skipping
/// comment subtrees and a comma directly before a closing delimiter, except
/// the only comma of a one-element tuple (see [`tuple_kinds`]). `text` is
/// the source the tree was parsed from.
pub fn semantic_stream(node: Node<'_>, text: &str) -> Vec<String> {
    stream_walk(node, text, &HashSet::new(), None)
        .0
        .to_strings()
}

/// A semantic stream held in one buffer: token `i` is
/// `buf[ends[i - 1] .. ends[i]]`. One allocation for the whole stream,
/// where a `Vec<String>` would make one per token.
#[derive(Debug, Default)]
struct Tokens {
    buf: String,
    ends: Vec<usize>,
}

impl Tokens {
    fn len(&self) -> usize {
        self.ends.len()
    }

    /// Close the token written to `buf` since the last one.
    fn end(&mut self) {
        self.ends.push(self.buf.len());
    }

    /// Tokens `[s, e)`.
    fn range(&self, s: usize, e: usize) -> impl Iterator<Item = &str> {
        (s..e).map(move |i| {
            let from = if i == 0 { 0 } else { self.ends[i - 1] };
            &self.buf[from..self.ends[i]]
        })
    }

    fn to_strings(&self) -> Vec<String> {
        self.range(0, self.len()).map(str::to_owned).collect()
    }
}

#[cfg(test)]
thread_local! {
    /// Work done on this thread by stream walks (nodes visited) and unit
    /// fingerprints (tokens hashed). Tests compare it across input sizes,
    /// so that a complexity check does not depend on machine speed.
    static WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_work(n: usize) {
    WORK.with(|w| w.set(w.get() + n));
}

#[cfg(not(test))]
#[inline(always)]
fn count_work(_: usize) {}

/// One step of [`stream_walk`]: emit a node, or close the token range of a
/// marked node after all its descendants.
enum Step<'t> {
    Enter(Node<'t>),
    Exit(usize),
}

/// The semantic stream of `node`, and for every node whose id is in `marks`
/// and that the walk reaches, its token range `[start, end)` in that
/// stream. The stream of a reached descendant is exactly that slice: a
/// pre-order walk visits a subtree contiguously, and the skip rules for the
/// nodes inside a subtree depend only on that subtree. (A marked node is
/// never itself a skipped node or a trailing comma.)
///
/// `omit`, when given, drops a node with its subtree as the walk reaches
/// it, after the trailing-comma decisions of its parent; it lets a test
/// compute a stream with parts left out by a direct walk.
fn stream_walk(
    node: Node<'_>,
    text: &str,
    marks: &HashSet<usize>,
    omit: Option<&dyn Fn(Node<'_>) -> bool>,
) -> (Tokens, HashMap<usize, (usize, usize)>) {
    let mut out = Tokens::default();
    let mut ranges = HashMap::new();
    let tuples = tuple_kinds(node.language().name());
    let mut cursor = node.walk();
    let mut stack = vec![Step::Enter(node)];
    while let Some(step) = stack.pop() {
        let n = match step {
            Step::Enter(n) => {
                count_work(1);
                n
            }
            Step::Exit(id) => {
                if let Some(r) = ranges.get_mut(&id) {
                    let r: &mut (usize, usize) = r;
                    r.1 = out.len();
                }
                continue;
            }
        };
        if is_skipped(n) || omit.is_some_and(|f| f(n)) {
            continue;
        }
        if marks.contains(&n.id()) {
            ranges.insert(n.id(), (out.len(), out.len()));
            stack.push(Step::Exit(n.id()));
        }
        let kind = n.kind();
        out.buf.push_str(kind);
        if n.child_count() == 0 {
            out.buf.push(':');
            out.buf.push_str(node_text(n, text));
        }
        out.end();
        if is_content_literal_kind(kind) {
            out.buf.push('=');
            out.buf.push_str(&text::normalize(node_text(n, text)));
            out.end();
        }
        // The children, comments and trailing commas dropped, go on the
        // stack in source order and are then reversed in place, so that the
        // stack pops them in source order. A comma is trailing when the next
        // child that is not skipped is a closing delimiter, so each child is
        // pushed once the one after it is known. Nothing is collected per
        // node: a node with a million children costs only its stack entries.
        let is_comma = |c: &Node<'_>| !c.is_named() && node_text(*c, text) == ",";
        // The only comma of a tuple-like node stays, even before the
        // closing delimiter: `(u8,)` is a tuple type, `(u8)` is `u8`.
        let tuple_comma =
            tuples.contains(&kind) && n.children(&mut cursor).filter(is_comma).count() == 1;
        let base = stack.len();
        let mut pending: Option<Node<'_>> = None;
        for c in n.children(&mut cursor) {
            if is_skipped(c) {
                continue;
            }
            if let Some(p) = pending {
                let trailing_comma = !tuple_comma
                    && is_comma(&p)
                    && !c.is_named()
                    && matches!(node_text(c, text), ")" | "]" | "}" | ">");
                if !trailing_comma {
                    stack.push(Step::Enter(p));
                }
            }
            pending = Some(c);
        }
        stack.extend(pending.map(Step::Enter));
        stack[base..].reverse();
    }
    (out, ranges)
}

/// The `ast-v1` fingerprint of `node` in a resource of class `class`.
pub fn ast_fingerprint(node: Node<'_>, text: &str, class: Class) -> Fingerprint {
    fingerprint::ast_v1(class.name(), &semantic_stream(node, text))
}

/// Section 16.1: trim WS from the span `[start, end)` (an empty result
/// becomes the one character that starts at `start`), take the smallest
/// node that covers it, and climb to the nearest named node. Offsets are
/// clamped to character boundaries of `text`.
pub fn smallest_named_node<'t>(root: Node<'t>, text: &str, start: usize, end: usize) -> Node<'t> {
    let start = text::floor_boundary(text, start);
    let end = text::ceil_boundary(text, end.max(start));
    let span = &text[start..end];
    let lead = span.len() - text::trim_start(span).len();
    let trimmed = text::trim(span);
    let (s, e) = if trimmed.is_empty() {
        let len = text[start..].chars().next().map_or(0, char::len_utf8);
        (start, start + len)
    } else {
        (start + lead, start + lead + trimmed.len())
    };
    // The covering node of the first to the last character: tree-sitter
    // takes the end of the range as a position that the node must reach.
    let last = text[s..e].char_indices().last().map_or(s, |(i, _)| s + i);
    let mut node = root.descendant_for_byte_range(s, last).unwrap_or(root);
    while !node.is_named() {
        match node.parent() {
            Some(p) => node = p,
            None => break,
        }
    }
    node
}

/// The byte ranges of every comment node (section 8.4) under `root`, in
/// source order. Comments nested in a comment are not listed separately.
pub fn comment_spans(root: Node<'_>) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let is_comment = is_comment_kind(node.kind());
        if is_comment {
            spans.push((node.start_byte(), node.end_byte()));
        }
        if !is_comment && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return spans;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Declaration tables (section 6.4)
// ---------------------------------------------------------------------------

/// How a declaration's name is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameRule {
    /// The text of the `name` field.
    Field,
    /// TS `variable_declarator`: `name` must be an `identifier`.
    TsDeclarator,
    /// Python `assignment`: `left` is an `identifier` and the parent is an
    /// `expression_statement`.
    PyAssignment,
    /// Rust `mod_item`: `name`, only with a `body`.
    RustMod,
    /// Rust `impl_item`: `impl T` or `impl Tr for T`.
    RustImpl,
    /// Go `method_declaration`: `R.name`.
    GoMethod,
    /// Java `variable_declarator` under `field_declaration` or
    /// `constant_declaration`.
    JavaDeclarator,
    /// A TypeScript enum member written as a bare name directly in an
    /// `enum_body`: the node's own text.
    TsEnumMember,
    /// Go `field_declaration`: the first `name` field, or for an embedded
    /// field its type name (without `*`, package qualifier and type
    /// arguments).
    GoField,
    /// Java `formal_parameter` or `spread_parameter` in the
    /// `formal_parameters` of a record. A varargs component (`String...
    /// rest`) has no `name` field: its name is the `name` field of its
    /// `variable_declarator`.
    JavaRecordComponent,
}

#[derive(Debug, Clone, Copy)]
struct Decl {
    unit_kind: &'static str,
    name: NameRule,
    bodies: &'static [&'static str],
}

const fn decl(unit_kind: &'static str, name: NameRule, bodies: &'static [&'static str]) -> Decl {
    Decl {
        unit_kind,
        name,
        bodies,
    }
}

fn wrappers(class: Class) -> &'static [&'static str] {
    match class {
        Class::Typescript | Class::Tsx => &[
            "export_statement",
            "ambient_declaration",
            "lexical_declaration",
            "variable_declaration",
        ],
        Class::Python => &["decorated_definition", "expression_statement"],
        // An extern block has no name; its items belong to the enclosing
        // module (ruling on section 6.4).
        Class::Rust => &["foreign_mod_item"],
        // `var_spec_list`: grouped `var (...)` (ruling on section 6.4).
        Class::Go => &[
            "type_declaration",
            "const_declaration",
            "var_declaration",
            "var_spec_list",
        ],
        Class::Java => &["field_declaration", "constant_declaration"],
        _ => &[],
    }
}

/// TypeScript and TSX: an `expression_statement` whose single named child is
/// an `internal_module` (a bare `namespace A {}`) is a wrapper; no other
/// expression statement is (ruling on section 6.4).
fn is_namespace_statement(class: Class, node: Node<'_>) -> bool {
    matches!(class, Class::Typescript | Class::Tsx)
        && node.kind() == "expression_statement"
        && node.named_child_count() == 1
        && node
            .named_child(0)
            .is_some_and(|c| c.kind() == "internal_module")
}

/// Body nodes whose named children are members of the same container
/// (Java `enum_body_declarations` inside `enum_body`).
fn inner_bodies(class: Class) -> &'static [&'static str] {
    match class {
        Class::Java => &["enum_body_declarations"],
        // The body of an extern block, looked through with the block.
        Class::Rust => &["declaration_list"],
        _ => &[],
    }
}

fn declaration(class: Class, kind: &str) -> Option<Decl> {
    use NameRule::*;
    Some(match (class, kind) {
        (
            Class::Typescript | Class::Tsx,
            "function_declaration" | "generator_function_declaration" | "function_signature",
        ) => decl("function", Field, &[]),
        (Class::Typescript | Class::Tsx, "class_declaration" | "abstract_class_declaration") => {
            decl("class", Field, &["class_body"])
        }
        (Class::Typescript | Class::Tsx, "interface_declaration") => {
            decl("interface", Field, &["interface_body"])
        }
        (Class::Typescript | Class::Tsx, "type_alias_declaration") => decl("type", Field, &[]),
        (Class::Typescript | Class::Tsx, "enum_declaration") => decl("enum", Field, &["enum_body"]),
        (Class::Typescript | Class::Tsx, "enum_assignment") => decl("variant", Field, &[]),
        (
            Class::Typescript | Class::Tsx,
            "property_identifier"
            | "private_property_identifier"
            | "string"
            | "number"
            | "computed_property_name",
        ) => decl("variant", TsEnumMember, &[]),
        (Class::Typescript | Class::Tsx, "internal_module" | "module") => {
            decl("module", Field, &["statement_block"])
        }
        (
            Class::Typescript | Class::Tsx,
            "method_definition" | "method_signature" | "abstract_method_signature",
        ) => decl("method", Field, &[]),
        (Class::Typescript | Class::Tsx, "public_field_definition" | "property_signature") => {
            decl("variable", Field, &[])
        }
        (Class::Typescript | Class::Tsx, "variable_declarator") => {
            decl("variable", TsDeclarator, &[])
        }

        (Class::Python, "function_definition") => decl("function", Field, &[]),
        (Class::Python, "class_definition") => decl("class", Field, &["block"]),
        (Class::Python, "assignment") => decl("variable", PyAssignment, &[]),

        (Class::Rust, "function_item" | "function_signature_item") => decl("function", Field, &[]),
        (Class::Rust, "struct_item" | "union_item") => {
            decl("struct", Field, &["field_declaration_list"])
        }
        (Class::Rust, "field_declaration") => decl("field", Field, &[]),
        (Class::Rust, "enum_item") => decl("enum", Field, &["enum_variant_list"]),
        (Class::Rust, "enum_variant") => decl("variant", Field, &[]),
        (Class::Rust, "trait_item") => decl("trait", Field, &["declaration_list"]),
        (Class::Rust, "impl_item") => decl("impl", RustImpl, &["declaration_list"]),
        (Class::Rust, "mod_item") => decl("module", RustMod, &["declaration_list"]),
        (Class::Rust, "type_item" | "associated_type") => decl("type", Field, &[]),
        (Class::Rust, "const_item" | "static_item") => decl("variable", Field, &[]),
        (Class::Rust, "macro_definition") => decl("macro", Field, &[]),

        (Class::Go, "function_declaration") => decl("function", Field, &[]),
        (Class::Go, "method_declaration") => decl("method", GoMethod, &[]),
        // A `type_spec` is a container through its `type` field (see
        // `bodies_of`).
        (Class::Go, "type_spec") => {
            decl("type", Field, &["field_declaration_list", "interface_type"])
        }
        (Class::Go, "type_alias") => decl("type", Field, &[]),
        (Class::Go, "field_declaration") => decl("field", GoField, &[]),
        (Class::Go, "method_elem") => decl("method", Field, &[]),
        (Class::Go, "const_spec" | "var_spec") => decl("variable", Field, &[]),

        (Class::Java, "class_declaration") => decl("class", Field, &["class_body"]),
        (Class::Java, "record_declaration") => {
            decl("class", Field, &["formal_parameters", "class_body"])
        }
        (Class::Java, "formal_parameter" | "spread_parameter") => {
            decl("field", JavaRecordComponent, &[])
        }
        (Class::Java, "enum_constant") => decl("variant", Field, &["class_body"]),
        (Class::Java, "interface_declaration" | "annotation_type_declaration") => decl(
            "interface",
            Field,
            &["interface_body", "annotation_type_body"],
        ),
        (Class::Java, "enum_declaration") => decl("enum", Field, &["enum_body"]),
        (Class::Java, "method_declaration" | "constructor_declaration") => {
            decl("method", Field, &[])
        }
        (Class::Java, "variable_declarator") => decl("variable", JavaDeclarator, &[]),
        // An element of an annotation type (ruling on section 6.4).
        (Class::Java, "annotation_type_element_declaration") => decl("method", Field, &[]),
        _ => return None,
    })
}

/// The member bodies of a container declaration: its named children whose
/// kind is in `bodies`, in source order. A Go `type_spec` reaches them
/// through its `type` field: the `field_declaration_list` of a struct type,
/// or an interface type itself.
fn bodies_of<'t>(node: Node<'t>, class: Class, bodies: &[&str]) -> Vec<Node<'t>> {
    if class == Class::Go && node.kind() == "type_spec" {
        let Some(ty) = node.child_by_field_name("type") else {
            return Vec::new();
        };
        return match ty.kind() {
            "interface_type" => vec![ty],
            "struct_type" => {
                let mut c = ty.walk();
                ty.named_children(&mut c)
                    .filter(|ch| ch.kind() == "field_declaration_list")
                    .collect()
            }
            _ => Vec::new(),
        };
    }
    let mut c = node.walk();
    node.named_children(&mut c)
        .filter(|ch| bodies.contains(&ch.kind()))
        .collect()
}

/// The symbol form of a type written in source (ruling on section 6.4, for
/// Rust `impl` types and traits and Go receivers): whitespace is dropped,
/// except that a run of whitespace between two characters that are each
/// alphanumeric, `_` or `'` becomes one space; the result is trimmed.
/// `dyn  Tr` gives `dyn Tr`, `& 'a G < u8 >` gives `&'a G<u8>`, `* Sp`
/// gives `*Sp`.
pub fn symbol_text(s: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let mut out = String::with_capacity(s.len());
    let mut gap = false;
    for c in s.chars() {
        if text::is_ws(c) {
            gap = true;
            continue;
        }
        if gap && out.chars().next_back().is_some_and(word) && word(c) {
            out.push(' ');
        }
        gap = false;
        out.push(c);
    }
    out
}

/// The own name of a declaration and the path prefix it gives its members
/// (they differ only for a Rust `impl_item`), or `None` when the node has
/// no name by its table and so is not a declaration unit.
fn decl_name(
    node: Node<'_>,
    parent: Option<Node<'_>>,
    rule: NameRule,
    text: &str,
) -> Option<(String, String)> {
    let field = |name: &str| node.child_by_field_name(name);
    let own = |s: String| Some((s.clone(), s));
    match rule {
        NameRule::Field => own(node_text(field("name")?, text).to_owned()),
        NameRule::TsDeclarator => {
            let name = field("name")?;
            (name.kind() == "identifier")
                .then(|| node_text(name, text).to_owned())
                .and_then(own)
        }
        NameRule::PyAssignment => {
            let left = field("left")?;
            (left.kind() == "identifier"
                && parent.is_some_and(|p| p.kind() == "expression_statement"))
            .then(|| node_text(left, text).to_owned())
            .and_then(own)
        }
        NameRule::RustMod => {
            field("body")?;
            own(node_text(field("name")?, text).to_owned())
        }
        NameRule::RustImpl => {
            let ty = symbol_text(node_text(field("type")?, text));
            let t = ty.split('<').next().unwrap_or("").to_owned();
            let name = match field("trait") {
                Some(tr) => format!("impl {} for {t}", symbol_text(node_text(tr, text))),
                None => format!("impl {t}"),
            };
            Some((name, t))
        }
        NameRule::GoMethod => {
            let receiver = field("receiver")?;
            let mut cursor = receiver.walk();
            let param = receiver
                .named_children(&mut cursor)
                .find(|c| c.kind() == "parameter_declaration")?;
            let ty = symbol_text(node_text(param.child_by_field_name("type")?, text));
            let ty = ty.strip_prefix('*').unwrap_or(&ty);
            let r = ty.split('[').next().unwrap_or("");
            own(format!("{r}.{}", node_text(field("name")?, text)))
        }
        NameRule::JavaDeclarator => {
            parent.filter(|p| matches!(p.kind(), "field_declaration" | "constant_declaration"))?;
            own(node_text(field("name")?, text).to_owned())
        }
        NameRule::TsEnumMember => {
            parent.filter(|p| p.kind() == "enum_body")?;
            own(node_text(node, text).to_owned())
        }
        NameRule::GoField => match field("name") {
            Some(n) => own(node_text(n, text).to_owned()),
            None => {
                // An embedded field: its name is the type name.
                let ty = symbol_text(node_text(field("type")?, text));
                let ty = ty.strip_prefix('*').unwrap_or(&ty);
                let ty = ty.split('[').next().unwrap_or("");
                let name = ty.rsplit('.').next().unwrap_or("");
                (!name.is_empty()).then(|| name.to_owned()).and_then(own)
            }
        },
        NameRule::JavaRecordComponent => {
            parent
                .filter(|p| p.kind() == "formal_parameters")
                .and_then(|p| p.parent())
                .filter(|r| r.kind() == "record_declaration")?;
            let name = if node.kind() == "spread_parameter" {
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .find(|c| c.kind() == "variable_declarator")?
                    .child_by_field_name("name")?
            } else {
                field("name")?
            };
            own(node_text(name, text).to_owned())
        }
    }
}

// ---------------------------------------------------------------------------
// Units (sections 6.4 to 6.6)
// ---------------------------------------------------------------------------

/// A unit of a code resource that parsed without error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeUnit {
    /// Section 6.6, duplicates numbered.
    pub key: String,
    /// A code kind of section 6.2, or `file`.
    pub kind: &'static str,
    /// The declaration path; `None` for `statement` and `file`.
    pub symbol: Option<String>,
    /// Key of the enclosing unit; `None` only for the `file` unit.
    pub parent: Option<String>,
    pub start: usize,
    pub end: usize,
    /// 1-based line of the first character.
    pub line: usize,
    /// 1-based line of the last character (of the first, for an empty range).
    pub end_line: usize,
    /// `ast-v1` of the unit node.
    pub fp: Fingerprint,
}

/// The units of a code resource that parsed without error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeFile {
    pub class: Class,
    /// Every unit in source order (start ascending, then end descending):
    /// the `file` unit first, a container before its members.
    pub units: Vec<CodeUnit>,
    /// The byte ranges of every comment node, in source order.
    pub comments: Vec<(usize, usize)>,
}

impl CodeFile {
    /// True when `[start, end)` intersects the range of a comment node
    /// (`in_comment` of section 6.7).
    pub fn in_comment(&self, start: usize, end: usize) -> bool {
        self.comments.iter().any(|&(s, e)| s < end && start < e)
    }

    /// The start of the first comment node that starts in `[from, to)`
    /// (for the default quote of section 6.7).
    pub fn first_comment_start(&self, from: usize, to: usize) -> Option<usize> {
        self.comments
            .iter()
            .map(|&(s, _)| s)
            .find(|&s| from <= s && s < to)
    }
}

/// A `block` unit of section 6.5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextUnit {
    /// `block:<12 hex>`, duplicates numbered.
    pub key: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub end_line: usize,
    /// `prose-v1`.
    pub fp: Fingerprint,
}

/// The units of a resource of a code class (sections 6.1 and 8.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Analysis {
    /// The file parsed without error: `ast-v1` units.
    Code(CodeFile),
    /// The parse tree has an error, or there is no tree: the file has no
    /// units (section 6.1). There is no fallback to block units for a code
    /// class.
    Unparsed,
}

/// One discovered declaration.
struct Found<'t> {
    node: Node<'t>,
    unit_node: Node<'t>,
    kind: &'static str,
    symbol: String,
    /// Index into the discovered declarations of the enclosing container.
    parent: Option<usize>,
    /// Index of the root named child it lies in.
    top: usize,
    /// The leading attributes or decorators that belong to the unit
    /// (ruling on section 6.4), in source order.
    lead: Vec<Node<'t>>,
    /// The declaration that declares this unit together with others: the
    /// outermost wrapper through which two or more units of this body were
    /// discovered (ruling on section 8.4). Its parts outside its first to
    /// last declarator units are shared by all of them.
    shared: Option<Node<'t>>,
    /// The leading nodes of `shared` (a Rust extern block's attributes).
    shared_lead: Vec<Node<'t>>,
    /// Go: for a `const_spec` without an expression, the nearest preceding
    /// `const_spec` of the same block that has one (implicit repetition,
    /// ruling on section 8.4).
    repeats: Option<Node<'t>>,
}

/// A container body to visit.
struct Body<'t> {
    node: Node<'t>,
    prefix: Option<String>,
    container: Option<usize>,
    /// The root named child this body lies in; `None` for the root itself.
    top: Option<usize>,
    /// The container depth of this body: 0 for the root, `d` for the body
    /// of a declaration at depth `d`.
    depth: usize,
}

/// The deepest container whose declarations are discovered. A declaration
/// at this depth (its path has this many segments) is a unit, but its body
/// is not searched: deeper declarations stay part of it (ruling on section
/// 6.4). This bounds the work of fingerprinting nested containers.
pub const MAX_CONTAINER_DEPTH: usize = 32;

/// The most tokens of shared declaration parts that a unit's stream takes
/// in (ruling on section 8.4). When the parts of a declaration outside its
/// declarator units are longer, each of its units has its own stream only;
/// a repeated Go `const_spec` longer than this is left out. This bounds the
/// work of fingerprinting declarations with many declarators.
pub const MAX_SHARED_TOKENS: usize = 256;

/// The node kind that, in a run immediately before a unit node, belongs to
/// that unit (ruling on section 6.4): Rust outer attributes and TypeScript
/// and TSX decorators that are siblings of a class member.
fn lead_kind(class: Class) -> Option<&'static str> {
    match class {
        Class::Rust => Some("attribute_item"),
        Class::Typescript | Class::Tsx => Some("decorator"),
        _ => None,
    }
}

/// The run of lead nodes immediately before `node`, comments between them
/// skipped, in source order.
fn leading<'t>(node: Node<'t>, class: Class) -> Vec<Node<'t>> {
    let Some(kind) = lead_kind(class) else {
        return Vec::new();
    };
    let mut lead = Vec::new();
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() == kind {
            lead.push(p);
        } else if !is_comment_kind(p.kind()) {
            break;
        }
        prev = p.prev_named_sibling();
    }
    lead.reverse();
    lead
}

/// Parse a file of a code class and build its units, or
/// [`Analysis::Unparsed`] when the tree has an error. For a class without a
/// grammar the result is [`Analysis::Unparsed`] as well.
pub fn analyze(text: &str, class: Class) -> Analysis {
    analyze_at_most(text, class, usize::MAX).expect("no unit limit")
}

/// [`analyze`], or `None` as soon as the result is known to have more than
/// `max_units` units, the `file` unit included. Work stops there: no unit
/// past the limit is fingerprinted or kept.
pub fn analyze_at_most(text: &str, class: Class, max_units: usize) -> Option<Analysis> {
    let Some(tree) = parse(text, class) else {
        return Some(Analysis::Unparsed);
    };
    let root = tree.root_node();
    if root.has_error() {
        return Some(Analysis::Unparsed);
    }
    code_units(root, text, class, max_units).map(Analysis::Code)
}

/// The line of the first parse error in `text` read as `class`: the first
/// error or missing node in source order. `None` when the text parses
/// without error or the class has no grammar.
pub fn first_error_line(text: &str, class: Class) -> Option<usize> {
    let tree = parse(text, class)?;
    let mut node = tree.root_node();
    if !node.has_error() {
        return None;
    }
    loop {
        if node.is_error() || node.is_missing() {
            return Some(node.start_position().row + 1);
        }
        let mut cursor = node.walk();
        let next = node
            .children(&mut cursor)
            .find(|c| c.is_error() || c.is_missing() || c.has_error())?;
        node = next;
    }
}

fn line_range(node: Node<'_>) -> (usize, usize) {
    let line = node.start_position().row + 1;
    let end = node.end_position();
    let end_line = if end.column == 0 && node.end_byte() > node.start_byte() {
        end.row
    } else {
        end.row + 1
    };
    (line, end_line.max(line))
}

fn code_units(root: Node<'_>, text: &str, class: Class, max_units: usize) -> Option<CodeFile> {
    // Every named root child that is not a comment or a lead node gives at
    // least one unit (a declaration in it, or itself as a statement), so
    // their count plus the `file` unit is a lower bound on the units. It
    // is checked before anything is built, and counting stops at the limit.
    let lead = lead_kind(class);
    let mut cursor = root.walk();
    let mut at_least = 1;
    for c in root.named_children(&mut cursor) {
        if !is_comment_kind(c.kind()) && Some(c.kind()) != lead {
            count_work(1);
            at_least += 1;
            if at_least > max_units {
                return None;
            }
        }
    }
    let found = discover(root, text, class, max_units.saturating_sub(1))?;

    // Statements: named root children that are not comments, not unit
    // nodes, and contain no unit node.
    let mut has_unit = vec![false; root.named_child_count()];
    for f in &found {
        has_unit[f.top] = true;
    }
    // A lead node (attribute, decorator) that belongs to a unit is not a
    // statement of its own. A lead node is not given leads itself, so a
    // run with no item after it stays as statements.
    // The leads of a shared declaration belong to its units as well.
    let mut absorbed: HashSet<usize> = found
        .iter()
        .flat_map(|f| f.lead.iter().chain(&f.shared_lead).map(|n| n.id()))
        .collect();
    let mut cursor = root.walk();
    let candidates: Vec<(Node<'_>, Vec<Node<'_>>)> = root
        .named_children(&mut cursor)
        .enumerate()
        .filter(|&(i, c)| !has_unit[i] && !is_comment_kind(c.kind()))
        .map(|(_, c)| {
            let l = if Some(c.kind()) == lead {
                Vec::new()
            } else {
                leading(c, class)
            };
            (c, l)
        })
        .collect();
    absorbed.extend(
        candidates
            .iter()
            .flat_map(|(_, l)| l.iter().map(|n| n.id())),
    );
    let statements: Vec<(Node<'_>, Vec<Node<'_>>)> = candidates
        .into_iter()
        .filter(|(c, _)| !absorbed.contains(&c.id()))
        .collect();
    if 1 + found.len() + statements.len() > max_units {
        return None;
    }

    // Every unit with its unit node and leads, then source order.
    enum Src {
        File,
        Decl(usize),
        Statement,
    }
    let start_of = |n: &Node<'_>, l: &[Node<'_>]| l.first().unwrap_or(n).start_byte();
    let mut all: Vec<(Node<'_>, Vec<Node<'_>>, Src)> =
        Vec::with_capacity(1 + found.len() + statements.len());
    all.push((root, Vec::new(), Src::File));
    all.extend(
        found
            .iter()
            .enumerate()
            .map(|(i, f)| (f.unit_node, f.lead.clone(), Src::Decl(i))),
    );
    all.extend(statements.into_iter().map(|(n, l)| (n, l, Src::Statement)));
    all[1..].sort_by_key(|(n, l, _)| (start_of(n, l), std::cmp::Reverse(n.end_byte())));

    // One walk of the root gives every unit's stream as a slice of the
    // root's stream, so nested containers are not walked once per level.
    // Shared declarations, their leads and repeated Go specs are marked too.
    let mut marks: HashSet<usize> = all
        .iter()
        .flat_map(|(n, l, _)| l.iter().chain(std::iter::once(n)).map(|x| x.id()))
        .collect();
    for f in &found {
        marks.extend(f.shared.iter().chain(&f.shared_lead).map(|n| n.id()));
        marks.extend(f.repeats.map(|n| n.id()));
    }
    let (root_stream, ranges) = stream_walk(root, text, &marks, None);

    // The token range of a unit node with its leads: one slice, because
    // only comment nodes, which emit nothing, lie between them.
    let span = |n: &Node<'_>, lead: &[Node<'_>]| -> Option<(usize, usize)> {
        let (s, _) = *ranges.get(&lead.first().unwrap_or(n).id())?;
        let (_, e) = *ranges.get(&n.id())?;
        Some((s, e))
    };
    // For each shared declaration: the first token of its first declarator
    // unit and the end of its last.
    let mut declarators: HashMap<usize, (usize, usize)> = HashMap::new();
    for f in &found {
        if let (Some(w), Some((s, e))) = (f.shared, span(&f.unit_node, &f.lead)) {
            let r = declarators.entry(w.id()).or_insert((s, e));
            *r = (r.0.min(s), r.1.max(e));
        }
    }
    // The token ranges whose concatenation is a unit's stream (ruling on
    // section 8.4): the shared declaration's tokens before its first
    // declarator unit, the repeated Go spec, the unit's own, and the
    // declaration's tokens after its last declarator unit. The shared parts
    // are left out when longer than MAX_SHARED_TOKENS.
    let pieces = |node: &Node<'_>, lead: &[Node<'_>], f: Option<&Found<'_>>| {
        let own = span(node, lead)?;
        let Some(f) = f else {
            return Some(vec![own]);
        };
        let mut pieces = Vec::with_capacity(4);
        let mut after = None;
        if let Some(w) = f.shared {
            let (ws, we) = span(&w, &f.shared_lead)?;
            let (first, last) = declarators[&w.id()];
            if (first - ws) + (we - last) <= MAX_SHARED_TOKENS {
                pieces.push((ws, first));
                after = Some((last, we));
            }
        }
        if let Some(r) = f.repeats {
            let (s, e) = *ranges.get(&r.id())?;
            if e - s <= MAX_SHARED_TOKENS {
                pieces.push((s, e));
            }
        }
        pieces.push(own);
        pieces.extend(after);
        Some(pieces)
    };

    let mut decl_pos = vec![0; found.len()];
    let mut units: Vec<CodeUnit> = Vec::with_capacity(all.len());
    for (node, lead, src) in &all {
        let f = match src {
            Src::Decl(i) => Some(&found[*i]),
            _ => None,
        };
        let fp = match pieces(node, lead, f) {
            Some(pieces) => {
                count_work(pieces.iter().map(|&(s, e)| e - s).sum());
                fingerprint::ast_v1_tokens(
                    class.name(),
                    pieces.iter().flat_map(|&(s, e)| root_stream.range(s, e)),
                )
            }
            None => {
                let mut stream = Vec::new();
                for n in lead.iter().chain(std::iter::once(node)) {
                    stream.extend(semantic_stream(*n, text));
                }
                count_work(stream.len());
                fingerprint::ast_v1(class.name(), &stream)
            }
        };
        let (kind, symbol, key) = match src {
            Src::File => ("file", None, "file".to_owned()),
            Src::Decl(i) => {
                decl_pos[*i] = units.len();
                let f = &found[*i];
                (
                    f.kind,
                    Some(f.symbol.clone()),
                    format!("{}:{}", f.kind, f.symbol),
                )
            }
            Src::Statement => (
                "statement",
                None,
                fingerprint::content_key("statement", &fp),
            ),
        };
        // The `file` unit has range [0, len) (section 6.2), which can be
        // wider than the root node; its fingerprint is still the root
        // node's (section 8.4).
        let (start, end, line, end_line) = match src {
            Src::File => {
                let last = text.char_indices().last().map_or(0, |(i, _)| i);
                (0, text.len(), 1, text::line_at(text, last))
            }
            _ => {
                let first = lead.first().unwrap_or(node);
                let (_, end_line) = line_range(*node);
                let line = first.start_position().row + 1;
                (first.start_byte(), node.end_byte(), line, end_line)
            }
        };
        units.push(CodeUnit {
            key,
            kind,
            symbol,
            parent: None,
            start,
            end,
            line,
            end_line,
            fp,
        });
    }
    let mut keys: Vec<String> = units.iter().map(|u| u.key.clone()).collect();
    fingerprint::number_duplicates(&mut keys);
    for (u, k) in units.iter_mut().zip(keys) {
        u.key = k;
    }
    for (pos, (_, _, src)) in all.iter().enumerate().skip(1) {
        let parent = match src {
            Src::Decl(i) => found[*i].parent.map(|p| units[decl_pos[p]].key.clone()),
            _ => None,
        };
        units[pos].parent = Some(parent.unwrap_or_else(|| "file".to_owned()));
    }
    Some(CodeFile {
        class,
        units,
        comments: comment_spans(root),
    })
}

/// Discovery of section 6.4, iterative. Declarations are returned in
/// pre-order (source order of their declaration nodes). `None` as soon as
/// more than `max_found` declarations are found.
fn discover<'t>(
    root: Node<'t>,
    text: &str,
    class: Class,
    max_found: usize,
) -> Option<Vec<Found<'t>>> {
    let wrappers = wrappers(class);
    let inner = inner_bodies(class);
    let mut found: Vec<Found<'t>> = Vec::new();
    let mut bodies = vec![Body {
        node: root,
        prefix: None,
        container: None,
        top: None,
        depth: 0,
    }];
    let mut cursor = root.walk();
    while let Some(body) = bodies.pop() {
        // Candidates of this body in source order, looking through wrappers
        // and inner bodies: (node, wrapper chain outermost first, top index).
        let mut pending: Vec<(Node<'t>, Vec<Node<'t>>, usize)> = body
            .node
            .named_children(&mut cursor)
            .enumerate()
            .map(|(i, c)| (c, Vec::new(), body.top.unwrap_or(i)))
            .collect();
        pending.reverse();
        let mut members: Vec<(usize, Vec<Node<'t>>)> = Vec::new();
        let mut nested: Vec<Body<'t>> = Vec::new();
        // Go: the last `const_spec` with an expression, with the id of the
        // block it is in. Candidates come in source order.
        let mut last_valued: Option<(Option<usize>, Node<'t>)> = None;
        while let Some((node, chain, top)) = pending.pop() {
            count_work(1);
            let kind = node.kind();
            let wrapper = wrappers.contains(&kind) || is_namespace_statement(class, node);
            if wrapper || inner.contains(&kind) {
                let mut sub = chain.clone();
                if wrapper {
                    sub.push(node);
                }
                let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
                pending.extend(children.into_iter().rev().map(|c| (c, sub.clone(), top)));
                continue;
            }
            let mut repeats = None;
            if class == Class::Go && kind == "const_spec" {
                let block = chain.last().map(|w| w.id());
                if node.child_by_field_name("value").is_some() {
                    last_valued = Some((block, node));
                } else {
                    repeats = last_valued.filter(|&(b, _)| b == block).map(|(_, n)| n);
                }
            }
            let Some(d) = declaration(class, kind) else {
                continue;
            };
            let parent = chain.last().copied().or(Some(body.node));
            let Some((name, member_prefix)) = decl_name(node, parent, d.name, text) else {
                continue;
            };
            let join = |n: &str| match &body.prefix {
                Some(p) => format!("{p}.{n}"),
                None => n.to_owned(),
            };
            let container_kind = body.container.map(|c| found[c].kind);
            let table_kind = if d.name == NameRule::TsDeclarator
                && node.child_by_field_name("value").is_some_and(|v| {
                    matches!(
                        v.kind(),
                        "arrow_function" | "function_expression" | "generator_function"
                    )
                }) {
                "function"
            } else {
                d.unit_kind
            };
            // In a class-like container a function is a method and a data
            // binding is a field (ruling on section 6.4); elsewhere they
            // stay `function` and `variable`.
            let class_like = matches!(
                container_kind,
                Some("class" | "interface" | "trait" | "impl" | "struct" | "enum" | "variant")
            );
            let unit_kind = match table_kind {
                "function" if class_like => "method",
                "variable" if class_like => "field",
                k => k,
            };
            let index = found.len();
            found.push(Found {
                node,
                unit_node: node,
                kind: unit_kind,
                symbol: join(&name),
                parent: body.container,
                top,
                lead: Vec::new(),
                shared: None,
                shared_lead: Vec::new(),
                repeats,
            });
            if found.len() > max_found {
                return None;
            }
            members.push((index, chain));
            // A declaration at depth MAX_CONTAINER_DEPTH is a unit, but its
            // body is not searched.
            if !d.bodies.is_empty() && body.depth + 1 < MAX_CONTAINER_DEPTH {
                for b in bodies_of(node, class, d.bodies) {
                    nested.push(Body {
                        node: b,
                        prefix: Some(join(&member_prefix)),
                        container: Some(index),
                        top: Some(top),
                        depth: body.depth + 1,
                    });
                }
            }
        }
        // Unit nodes: climb through a wrapper while this is the only
        // declaration discovered through it.
        let mut through: HashMap<usize, usize> = HashMap::new();
        let mut shared_lead: HashMap<usize, Vec<Node<'t>>> = HashMap::new();
        for (_, chain) in &members {
            for w in chain {
                *through.entry(w.id()).or_default() += 1;
            }
        }
        for (index, chain) in &members {
            let mut unit_node = found[*index].node;
            for w in chain.iter().rev() {
                if through[&w.id()] == 1 {
                    unit_node = *w;
                } else {
                    break;
                }
            }
            found[*index].unit_node = unit_node;
            found[*index].lead = leading(unit_node, class);
            // The outermost wrapper counts the most declarations, so it is
            // the declaration when any wrapper of the chain is shared.
            if let Some(&w) = chain.first().filter(|w| through[&w.id()] >= 2) {
                let lead = shared_lead
                    .entry(w.id())
                    .or_insert_with(|| leading(w, class));
                found[*index].shared = Some(w);
                found[*index].shared_lead = lead.clone();
            }
        }
        // Visit nested bodies in source order.
        bodies.extend(nested.into_iter().rev());
    }
    // `bodies` is a stack, so containers are visited depth first, but
    // members were appended after all declarations of the outer body; sort
    // into pre-order of declaration nodes, keeping parent indices valid.
    let mut order: Vec<usize> = (0..found.len()).collect();
    order.sort_by_key(|&i| {
        (
            found[i].node.start_byte(),
            std::cmp::Reverse(found[i].node.end_byte()),
        )
    });
    let mut new_index = vec![0; found.len()];
    for (new, &old) in order.iter().enumerate() {
        new_index[old] = new;
    }
    let mut slots: Vec<Option<Found<'t>>> = found.into_iter().map(Some).collect();
    Some(
        order
            .iter()
            .map(|&old| {
                let mut f = slots[old].take().expect("each index once");
                f.parent = f.parent.map(|p| new_index[p]);
                f
            })
            .collect(),
    )
}

/// The `block` units of section 6.5 for class `text`: each maximal run of
/// lines that are not empty after trimming, from the first non-WS character
/// to the last, with `prose-v1` fingerprints and numbered keys. The `file`
/// unit is not included.
pub fn text_units(text: &str) -> Vec<TextUnit> {
    text_units_at_most(text, usize::MAX).expect("no block limit")
}

/// [`text_units`], or `None` as soon as there are more than `max_blocks`
/// blocks. Nothing is fingerprinted before the count is known.
pub fn text_units_at_most(text: &str, max_blocks: usize) -> Option<Vec<TextUnit>> {
    let mut spans: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut run: Option<(usize, usize, usize, usize)> = None;
    let mut start = 0;
    for (n, line) in text.split('\n').enumerate() {
        count_work(1);
        let lineno = n + 1;
        let t = text::trim(line);
        if t.is_empty() {
            spans.extend(run.take());
            if spans.len() > max_blocks {
                return None;
            }
        } else {
            let lead = line.len() - text::trim_start(line).len();
            let end = start + text::trim_end(line).len();
            match &mut run {
                Some(r) => {
                    r.1 = end;
                    r.3 = lineno;
                }
                None => run = Some((start + lead, end, lineno, lineno)),
            }
        }
        start += line.len() + 1;
    }
    spans.extend(run.take());
    if spans.len() > max_blocks {
        return None;
    }
    let fps: Vec<Fingerprint> = spans
        .iter()
        .map(|&(s, e, _, _)| fingerprint::prose_v1(&text[s..e]))
        .collect();
    let mut keys: Vec<String> = fps
        .iter()
        .map(|fp| fingerprint::content_key("block", fp))
        .collect();
    fingerprint::number_duplicates(&mut keys);
    Some(
        spans
            .into_iter()
            .zip(fps)
            .zip(keys)
            .map(|(((start, end, line, end_line), fp), key)| TextUnit {
                key,
                start,
                end,
                line,
                end_line,
                fp,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(text: &str, class: Class) -> CodeFile {
        match analyze(text, class) {
            Analysis::Code(f) => f,
            Analysis::Unparsed => panic!("{text:?} did not parse as {class:?}"),
        }
    }

    fn unit_fp(f: &CodeFile, key: &str) -> Fingerprint {
        f.units
            .iter()
            .find(|u| u.key == key)
            .unwrap_or_else(|| panic!("no unit {key}"))
            .fp
            .clone()
    }

    /// The units of `text` and the work counted while building them.
    fn with_work(text: &str, class: Class) -> (CodeFile, usize) {
        WORK.with(|w| w.set(0));
        let f = code(text, class);
        (f, WORK.with(|w| w.get()))
    }

    /// Doubling the input must at most about double the work: a linear
    /// algorithm gives a ratio near 2, a quadratic one near 4. The bound 3
    /// separates them without depending on machine speed or load.
    fn assert_linear(small: usize, large: usize, what: &str) {
        assert!(
            large <= 3 * small,
            "{what}: work grew from {small} to {large} when the input doubled"
        );
    }

    /// Ruling on section 6.4 (MAX_CONTAINER_DEPTH): containers nested deep
    /// give 32 declaration units, and the work (nodes walked plus tokens
    /// fingerprinted) grows linearly with the depth.
    #[test]
    fn deep_container_nesting_is_bounded() {
        let ts = |n: usize| format!("{}{}", "namespace A {\n".repeat(n), "}\n".repeat(n));
        let rs = |n: usize| format!("{}{}", "mod a {\n".repeat(n), "}\n".repeat(n));
        for (class, make) in [
            (Class::Typescript, &ts as &dyn Fn(usize) -> String),
            (Class::Rust, &rs),
        ] {
            let (_, small) = with_work(&make(2000), class);
            let (f, large) = with_work(&make(4000), class);
            assert_linear(small, large, &format!("{class:?} nesting"));
            let decls = f.units.iter().filter(|u| u.symbol.is_some()).count();
            assert_eq!(decls, MAX_CONTAINER_DEPTH, "{class:?}");
            let deepest = f
                .units
                .iter()
                .filter_map(|u| u.symbol.as_ref())
                .max_by_key(|s| s.len())
                .unwrap();
            assert_eq!(deepest.split('.').count(), MAX_CONTAINER_DEPTH);
        }
    }

    /// The stream of a discovered unit by the rules of section 8.4, from
    /// direct walks rather than slices of the root's stream: its leads and
    /// unit node; before them, the repeated Go spec; and, for a declarator
    /// of a shared declaration, the declaration's walk (after its leads)
    /// with every node that lies between its first and last declarator
    /// units left out, except the unit itself and the nodes around it.
    fn direct_stream(d: &Found<'_>, found: &[Found<'_>], text: &str) -> Vec<String> {
        let walk = |n: &Node<'_>| semantic_stream(*n, text);
        let own: Vec<String> = d.lead.iter().chain([&d.unit_node]).flat_map(walk).collect();
        let rep: Vec<String> = d
            .repeats
            .iter()
            .map(walk)
            .find(|s| s.len() <= MAX_SHARED_TOKENS)
            .unwrap_or_default();
        let Some(w) = d.shared else {
            return [rep, own].concat();
        };
        let group = || found.iter().filter(|f| f.shared == Some(w));
        let first = group()
            .map(|f| f.lead.first().unwrap_or(&f.unit_node).start_byte())
            .min()
            .unwrap();
        let last = group().map(|f| f.unit_node.end_byte()).max().unwrap();
        let (own_start, own_end) = (
            d.lead.first().unwrap_or(&d.unit_node).start_byte(),
            d.unit_node.end_byte(),
        );
        let mut around = HashSet::new();
        let mut p = d.unit_node.parent();
        while let Some(n) = p.filter(|n| n.id() != w.id()) {
            around.insert(n.id());
            p = n.parent();
        }
        let inside_own = |n: Node<'_>| own_start <= n.start_byte() && n.end_byte() <= own_end;
        let middle = |n: Node<'_>| {
            first <= n.start_byte()
                && n.end_byte() <= last
                && !around.contains(&n.id())
                && !inside_own(n)
        };
        let before = |n: Node<'_>| first <= n.start_byte() && !around.contains(&n.id());
        let lead: Vec<String> = d.shared_lead.iter().flat_map(walk).collect();
        let none = HashSet::new();
        let walk_omit = |omit: &dyn Fn(Node<'_>) -> bool| {
            stream_walk(w, text, &none, Some(omit)).0.to_strings()
        };
        let prefix = [lead.clone(), walk_omit(&before)].concat();
        let whole = [lead, walk_omit(&middle)].concat();
        let suffix = whole[prefix.len() + own.len()..].to_vec();
        assert_eq!(whole[prefix.len()..prefix.len() + own.len()], own[..]);
        if prefix.len() + suffix.len() > MAX_SHARED_TOKENS {
            [rep, own].concat()
        } else {
            [prefix, rep, own, suffix].concat()
        }
    }

    /// The one-walk fingerprints equal the fingerprints of each unit's
    /// stream computed by direct walks (see [`direct_stream`]), on every
    /// units-code fixture.
    #[test]
    fn slice_fingerprints_equal_direct_streams() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/fixtures/units");
        let (mut checked, mut shared, mut repeated) = (0, 0, 0);
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let name = path.file_name().unwrap().to_str().unwrap();
            for class in [Class::from_path(name), Class::Tsx] {
                if !class.is_code() || (class == Class::Tsx && !name.ends_with(".ts")) {
                    continue;
                }
                let Some(tree) = parse(&text, class) else {
                    continue;
                };
                let root = tree.root_node();
                if root.has_error() {
                    continue;
                }
                let found = discover(root, &text, class, usize::MAX).expect("no limit");
                let f = code_units(root, &text, class, usize::MAX).expect("no limit");
                for d in &found {
                    let stream = direct_stream(d, &found, &text);
                    shared += usize::from(d.shared.is_some());
                    repeated += usize::from(d.repeats.is_some());
                    let start = d.lead.first().unwrap_or(&d.unit_node).start_byte();
                    let u = f
                        .units
                        .iter()
                        .find(|u| {
                            u.start == start
                                && u.end == d.unit_node.end_byte()
                                && u.symbol.as_deref() == Some(&d.symbol)
                        })
                        .unwrap_or_else(|| panic!("{name}: {}", d.symbol));
                    assert_eq!(
                        u.fp,
                        fingerprint::ast_v1(class.name(), &stream),
                        "{name}: {}",
                        d.symbol
                    );
                    checked += 1;
                }
                assert_eq!(f.units[0].fp, ast_fingerprint(root, &text, class), "{name}");
            }
        }
        assert!(checked > 100, "{checked}");
        assert!(shared > 10 && repeated > 0, "{shared} {repeated}");
    }

    /// Ruling 14: a struct with 5,000 fields and an enum with 5,000
    /// variants are analyzed in linear time (each member fingerprints only
    /// itself, the container once).
    #[test]
    fn many_members_stay_linear() {
        let make = |n: usize| {
            let fields: String = (0..n).map(|i| format!("    f{i}: u8,\n")).collect();
            let variants: String = (0..n).map(|i| format!("    V{i},\n")).collect();
            format!("struct S {{\n{fields}}}\nenum E {{\n{variants}}}\n")
        };
        let (_, small) = with_work(&make(2500), Class::Rust);
        let (f, large) = with_work(&make(5000), Class::Rust);
        assert_eq!(f.units.len(), 1 + 2 + 2 * 5000);
        assert_eq!(unit_fp(&f, "field:S.f4999").profile(), "ast-v1");
        assert_linear(small, large, "members");
    }

    /// Ruling 2: leading attributes and decorators are part of the unit's
    /// range and fingerprint; a statement does not form for them.
    #[test]
    fn leading_attributes_and_decorators_belong_to_the_unit() {
        let a = code(
            "// c\n#[derive(Debug)]\n/// doc\n#[repr(C)]\npub struct S;\n",
            Class::Rust,
        );
        let b = code(
            "// c\n#[derive(Clone)]\n/// doc\n#[repr(C)]\npub struct S;\n",
            Class::Rust,
        );
        assert_ne!(unit_fp(&a, "struct:S"), unit_fp(&b, "struct:S"));
        assert!(a.units.iter().all(|u| u.kind != "statement"));
        let s = a.units.iter().find(|u| u.key == "struct:S").unwrap();
        assert_eq!((s.start, s.line), (5, 2));
        // Comments between the attributes do not count.
        let c = code(
            "// c\n#[derive(Debug)]\n#[repr(C)]\npub struct S;\n",
            Class::Rust,
        );
        assert_eq!(unit_fp(&a, "struct:S"), unit_fp(&c, "struct:S"));
        // An inner attribute stays a statement.
        let d = code("#![allow(x)]\nfn f() {}\n", Class::Rust);
        assert_eq!(d.units[1].kind, "statement");

        let m = |dec: &str| {
            let f = code(
                &format!("class C {{\n  {dec}\n  m() {{}}\n}}\n"),
                Class::Typescript,
            );
            unit_fp(&f, "method:C.m")
        };
        assert_ne!(m("@log"), m("@trace"));
        assert_eq!(m("@log"), m("@log // why"));
    }

    /// Ruling 4: a JSX expression container with only comments is skipped.
    #[test]
    fn comment_only_jsx_expressions_are_skipped() {
        let fp = |body: &str| {
            let f = code(&format!("const v = <div>{body}</div>;\n"), Class::Tsx);
            unit_fp(&f, "variable:v")
        };
        assert_eq!(fp("{/* j */}"), fp(""));
        assert_eq!(fp("{}"), fp(""));
        assert_eq!(fp("{/* a */ /* b */}"), fp(""));
        assert_ne!(fp("{x}"), fp(""));
        assert_ne!(fp("{/* j */ x}"), fp(""));
    }

    /// Ruling 3: symbol text normalisation.
    #[test]
    fn symbol_text_keeps_one_space_between_words() {
        assert_eq!(symbol_text("dyn  Tr"), "dyn Tr");
        assert_eq!(symbol_text(" & 'a  G < u8 > "), "&'a G<u8>");
        assert_eq!(symbol_text("* Sp"), "*Sp");
        assert_eq!(symbol_text("G2 [T]"), "G2[T]");
        assert_eq!(symbol_text("Iterator<Item = u8>"), "Iterator<Item=u8>");
        assert_eq!(symbol_text("a_b\n\tc'd"), "a_b c'd");
    }

    /// R49: a file nested 100,000 levels deep is processed or reported as
    /// class `text` without a crash. Runs on a thread with a small stack so
    /// that a recursive walk would overflow.
    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        const DEPTH: usize = 100_000;
        let open = "(".repeat(DEPTH);
        let close = ")".repeat(DEPTH);
        let sources = [
            (Class::Typescript, format!("const x = {open}1{close};\n")),
            (Class::Tsx, format!("const x = {open}1{close};\n")),
            (Class::Python, format!("x = {open}1{close}\n")),
            (Class::Rust, format!("const X: i32 = {open}1{close};\n")),
            (Class::Go, format!("package p\n\nvar x = {open}1{close}\n")),
            (
                Class::Java,
                format!("class A {{ int x = {open}1{close}; }}\n"),
            ),
        ];
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                for (class, src) in sources {
                    match analyze(&src, class) {
                        Analysis::Code(f) => assert!(f.units.len() >= 2, "{class:?}"),
                        Analysis::Unparsed => {}
                    }
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn fp_of(text: &str, class: Class, key: &str) -> Fingerprint {
        let f = code(text, class);
        f.units
            .into_iter()
            .find(|u| u.key == key)
            .unwrap_or_else(|| panic!("no unit {key}"))
            .fp
    }

    /// Section 8.4: whitespace, line breaks, comments and a comma before a
    /// closing delimiter do not change the fingerprint; a literal,
    /// identifier, operator or keyword does.
    #[test]
    fn ast_v1_properties() {
        let base = fp_of(
            "function f(a, b) { return a + 1; }",
            Class::Typescript,
            "function:f",
        );
        for same in [
            "function   f(a,\n  b)  {\n  return a+1;\n}",
            "function f(a, b,) { /* x */ return a + 1; // y\n}",
        ] {
            assert_eq!(fp_of(same, Class::Typescript, "function:f"), base, "{same}");
        }
        for changed in [
            "function f(a, b) { return a + 2; }",
            "function f(a, c) { return a + 1; }",
            "function f(a, b) { return a - 1; }",
            "function f(a, b) { return (a + 1); }",
        ] {
            assert_ne!(
                fp_of(changed, Class::Typescript, "function:f"),
                base,
                "{changed}"
            );
        }
    }

    /// Ruling on section 8.4: the only comma of a Rust or Python tuple-like
    /// node counts, so `(u8,)` and `(u8)` differ; any other comma before a
    /// closing delimiter, and a second one in a tuple, is still skipped.
    #[test]
    fn one_element_tuple_commas_count() {
        let rs = |src: &str| fp_of(src, Class::Rust, "type:A");
        assert_ne!(rs("type A = (u8,);"), rs("type A = (u8);"));
        assert_eq!(rs("type A = (u8, u16,);"), rs("type A = (u8, u16);"));
        let py = |src: &str| fp_of(src, Class::Python, "variable:v");
        assert_ne!(py("v = x[1,]"), py("v = x[1]"));
        assert_eq!(py("v = f(a,)"), py("v = f(a)"));
        assert_eq!(py("v = [1,]"), py("v = [1]"));
        // A TypeScript tuple type is not tuple-like: `[number,]` is `[number]`.
        let ts = |src: &str| fp_of(src, Class::Typescript, "type:A");
        assert_eq!(ts("type A = [number,];"), ts("type A = [number];"));
    }

    /// Ruling on section 8.4: the parts of a declaration outside its
    /// declarator units (type, modifiers, keyword, annotations) reach every
    /// declarator; editing one declarator, or adding one, leaves the others.
    #[test]
    fn shared_declaration_parts_reach_every_declarator() {
        let java = |src: &str, key: &str| fp_of(&format!("class K {{ {src} }}"), Class::Java, key);
        for key in ["field:K.a", "field:K.b"] {
            assert_ne!(java("int a, b = 2;", key), java("long a, b = 2;", key));
            assert_ne!(java("int a, b = 2;", key), java("@A int a, b = 2;", key));
        }
        assert_eq!(
            java("int a, b = 2;", "field:K.a"),
            java("int a, b = 3;", "field:K.a")
        );
        assert_ne!(
            java("int a, b = 2;", "field:K.b"),
            java("int a, b = 3;", "field:K.b")
        );

        for class in [Class::Typescript, Class::Tsx] {
            let ts = |src: &str, key: &str| fp_of(src, class, key);
            for key in ["variable:a", "variable:b"] {
                assert_ne!(ts("const a = 1, b = 2;", key), ts("let a = 1, b = 2;", key));
                assert_ne!(
                    ts("const a = 1, b = 2;", key),
                    ts("export const a = 1, b = 2;", key)
                );
            }
            assert_eq!(
                ts("const a = 1, b = 2;", "variable:a"),
                ts("const a = 1, b = 3;", "variable:a")
            );
            assert_eq!(
                ts("const a = 1, b = 2;", "variable:b"),
                ts("const a = 0, c = 5, b = 2;", "variable:b")
            );
        }

        let go = |src: &str, key: &str| fp_of(&format!("package p\n\n{src}\n"), Class::Go, key);
        let vars = "var (\n\ta = 1\n\tb = 2\n)";
        assert_eq!(
            go(vars, "variable:a"),
            go("var (\n\ta = 1\n\tc = 3\n\tb = 2\n)", "variable:a")
        );
        assert_eq!(
            go(vars, "variable:a"),
            go("var (\n\ta = 1\n\tb = 5\n)", "variable:a")
        );
        assert_ne!(
            go(vars, "variable:b"),
            go("var (\n\ta = 1\n\tb = 5\n)", "variable:b")
        );
        // The group's parentheses are shared: `a` grouped is not `a` alone.
        assert_ne!(
            go(vars, "variable:a"),
            go("var a = 1\nvar b = 2", "variable:a")
        );
        let types = "type (\n\tT int\n\tU string\n)";
        assert_eq!(
            go(types, "type:T"),
            go("type (\n\tT int\n\tU bool\n)", "type:T")
        );
    }

    /// Ruling on section 8.4: a Rust extern block with several items is
    /// shared by them: its ABI string and leading attributes reach every
    /// item, and the attributes are not a statement.
    #[test]
    fn extern_block_parts_reach_every_item() {
        let rs = |src: &str, key: &str| fp_of(src, Class::Rust, key);
        let base = "#[link(name = \"m\")]\nextern \"C\" {\n    fn cos(x: f64) -> f64;\n    fn sin(x: f64) -> f64;\n}\n";
        for key in ["function:cos", "function:sin"] {
            assert_ne!(rs(base, key), rs(&base.replace("\"C\"", "\"system\""), key));
            assert_ne!(rs(base, key), rs(&base.replace("\"m\"", "\"n\""), key));
        }
        assert_eq!(
            rs(base, "function:cos"),
            rs(&base.replace("sin(x: f64)", "sin(y: f64)"), "function:cos")
        );
        assert!(
            code(base, Class::Rust)
                .units
                .iter()
                .all(|u| u.kind != "statement")
        );
    }

    /// Ruling on section 8.4: a Go `const_spec` without an expression takes
    /// in the stream of the nearest preceding spec of its block that has
    /// one.
    #[test]
    fn go_implicit_repetition_reaches_repeating_specs() {
        let go = |src: &str, key: &str| {
            fp_of(&format!("package p\n\nconst (\n{src}\n)\n"), Class::Go, key)
        };
        let base = "\tA = iota * 10\n\tB\n\tC";
        let edited = "\tA = iota * 100\n\tB\n\tC";
        for key in ["variable:B", "variable:C"] {
            assert_ne!(go(base, key), go(edited, key));
        }
        assert_eq!(
            go(base, "variable:C"),
            go("\tA = iota * 10\n\tD\n\tC", "variable:C")
        );
        // The nearest spec with an expression, not the first one.
        let two = "\tA = iota\n\tB = iota * 2\n\tC";
        assert_eq!(
            go(two, "variable:C"),
            go("\tA = iota + 7\n\tB = iota * 2\n\tC", "variable:C")
        );
        assert_ne!(
            go(two, "variable:C"),
            go("\tA = iota\n\tB = iota * 3\n\tC", "variable:C")
        );
    }

    /// `const {p0, ..., p(k-1)} = <value>, a = 1, b = 2;` in TypeScript.
    fn destructured(k: usize, value: &str, keyword: &str) -> String {
        let names: Vec<String> = (0..k).map(|i| format!("p{i}")).collect();
        format!(
            "{keyword} {{{}}} = {value}, a = 1, b = 2;\n",
            names.join(", ")
        )
    }

    /// MAX_SHARED_TOKENS: shared parts of exactly 256 tokens are kept, 257
    /// are left out. The shared tokens of `destructured(k, value)` are
    /// `lexical_declaration`, the keyword, the destructuring declarator
    /// (`variable_declarator`, `object_pattern`, `{`, k names, k - 1
    /// commas, `}`, `=` and the value) and its comma before `a`, and `;`
    /// after `b`: 2k + 8 plus the value's tokens. `o` is one token,
    /// `o.q` four (`member_expression`, `o`, `.`, `q`).
    #[test]
    fn shared_parts_are_capped() {
        let fp = |k: usize, value: &str, keyword: &str| {
            fp_of(
                &destructured(k, value, keyword),
                Class::Typescript,
                "variable:b",
            )
        };
        assert_eq!(2 * 122 + 8 + 4, MAX_SHARED_TOKENS);
        assert_ne!(fp(122, "o.q", "const"), fp(122, "o.q", "let"));
        assert_eq!(2 * 124 + 8 + 1, MAX_SHARED_TOKENS + 1);
        assert_eq!(fp(124, "o", "const"), fp(124, "o", "let"));
        // Over the cap the unit's stream is its own.
        let own = [
            "variable_declarator",
            "identifier:b",
            "=:=",
            "number:2",
            "=2",
        ];
        assert_eq!(
            fp(124, "o", "const"),
            fingerprint::ast_v1("typescript", &own)
        );
    }

    /// MAX_SHARED_TOKENS for a repeated Go spec. `A = e` with e a sum of m
    /// operands has the stream `const_spec`, `A`, `=`, `expression_list`,
    /// then m - 1 `binary_expression` and m - 1 `+` tokens and one token
    /// per identifier operand, two per `int_literal` (its text and `=`):
    /// with m identifiers 3m + 2 tokens, one more per literal.
    #[test]
    fn repeated_spec_is_capped() {
        let sum = |literals: usize, m: usize, last: &str| {
            let mut ops: Vec<String> = (0..literals).map(|i| i.to_string()).collect();
            ops.extend((literals..m - 1).map(|_| "x".to_owned()));
            ops.push(last.to_owned());
            ops.join(" + ")
        };
        let fp = |e: String| {
            fp_of(
                &format!("package p\n\nconst (\n\tA = {e}\n\tB\n)\n"),
                Class::Go,
                "variable:B",
            )
        };
        assert_eq!(3 * 84 + 2 + 2, MAX_SHARED_TOKENS);
        assert_ne!(fp(sum(2, 84, "x")), fp(sum(2, 84, "y")));
        assert_eq!(3 * 85 + 2, MAX_SHARED_TOKENS + 1);
        assert_eq!(fp(sum(0, 85, "x")), fp(sum(0, 85, "y")));
    }

    /// The shared parts and repeated specs keep the work linear: thousands
    /// of declarators in one declaration, in TypeScript, Java and Go, a Go
    /// constant group where every spec repeats the first, and a shared
    /// prefix that grows with the number of declarators (a destructuring
    /// declarator before them), which the cap leaves out.
    #[test]
    fn shared_declarations_stay_linear() {
        let list = |n: usize, f: &dyn Fn(usize) -> String, sep: &str| {
            (0..n).map(f).collect::<Vec<_>>().join(sep)
        };
        let ts = |n: usize| format!("const {};\n", list(n, &|i| format!("a{i} = {i}"), ", "));
        let java = |n: usize| {
            format!(
                "class K {{ int {}; }}\n",
                list(n, &|i| format!("a{i}"), ", ")
            )
        };
        let go_var = |n: usize| {
            format!(
                "package p\n\nvar (\n{}\n)\n",
                list(n, &|i| format!("\ta{i} = {i}"), "\n")
            )
        };
        let go_iota = |n: usize| {
            format!(
                "package p\n\nconst (\n\tA = iota * 10\n{}\n)\n",
                list(n, &|i| format!("\tB{i}"), "\n")
            )
        };
        let crafted = |n: usize| {
            format!(
                "const {{{}}} = o, {};\n",
                list(n, &|i| format!("p{i}"), ", "),
                list(n, &|i| format!("a{i} = {i}"), ", ")
            )
        };
        for (class, make, what) in [
            (Class::Typescript, &ts as &dyn Fn(usize) -> String, "ts"),
            (Class::Java, &java, "java"),
            (Class::Go, &go_var, "go var"),
            (Class::Go, &go_iota, "go iota"),
            (Class::Typescript, &crafted, "crafted prefix"),
        ] {
            let (_, small) = with_work(&make(5000), class);
            let (f, large) = with_work(&make(10000), class);
            assert!(f.units.len() > 10000, "{what}");
            assert_linear(small, large, what);
        }
    }

    #[test]
    fn a_parse_error_gives_no_units() {
        assert_eq!(
            analyze("def f(:\n    pass\n\n\n  x = 1  \n", Class::Python),
            Analysis::Unparsed
        );
    }

    #[test]
    fn text_blocks_are_runs_of_non_blank_lines() {
        let units = text_units("def f(:\n    pass\n\n\n  x = 1  \n");
        let spans: Vec<(usize, usize, usize, usize)> = units
            .iter()
            .map(|u| (u.start, u.end, u.line, u.end_line))
            .collect();
        assert_eq!(spans, [(0, 16, 1, 2), (21, 26, 5, 5)]);
        assert!(
            units
                .iter()
                .all(|u| u.key.starts_with("block:") && u.fp.profile() == "prose-v1")
        );
    }

    fn stream_of(src: &str, class: Class) -> Vec<String> {
        let tree = parse(src, class).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        semantic_stream(tree.root_node(), src)
    }

    /// Rust `doc_comment` occurs only inside `line_comment` or
    /// `block_comment`, which are skipped whole.
    #[test]
    fn rust_doc_comments_are_skipped_with_their_comment() {
        let tree = parse("/// a\n//! b\n/** c */\nfn f() {}", Class::Rust).unwrap();
        let mut cursor = tree.walk();
        let mut doc_parents = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            if n.kind() == "doc_comment" {
                doc_parents.push(n.parent().unwrap().kind().to_owned());
            }
            stack.extend(n.children(&mut cursor));
        }
        assert_eq!(doc_parents.len(), 3);
        assert!(
            doc_parents
                .iter()
                .all(|k| k == "line_comment" || k == "block_comment")
        );
        let a = stream_of("impl A {\n    /// one\n    fn f() {}\n}", Class::Rust);
        let b = stream_of(
            "impl A {\n    /// two\n    /** x */\n    fn f() {}\n}",
            Class::Rust,
        );
        assert_eq!(a, b);
        assert!(
            !a.iter()
                .any(|t| t.contains("doc_comment") || t.contains("one"))
        );
    }

    /// TypeScript and TSX `html_comment` is a comment kind.
    #[test]
    fn html_comments_are_skipped() {
        for class in [Class::Typescript, Class::Tsx] {
            let with = "const a = 1;\n<!-- legacy\nconst b = 2;\n";
            let tree = parse(with, class).unwrap();
            assert!(
                tree.root_node().to_sexp().contains("html_comment"),
                "{}",
                tree.root_node().to_sexp()
            );
            assert_eq!(
                stream_of(with, class),
                stream_of("const a = 1;\nconst b = 2;\n", class)
            );
        }
    }

    fn keys(src: &str, class: Class) -> Vec<(String, usize, usize)> {
        code(src, class)
            .units
            .into_iter()
            .map(|u| (u.key, u.start, u.end))
            .collect()
    }

    /// A bare `namespace A {}` is `expression_statement > internal_module`.
    #[test]
    fn bare_namespace_is_a_module() {
        let src = "namespace A { export function f() {} }\nfoo();\n";
        let k = keys(src, Class::Typescript);
        assert_eq!(k[1], ("module:A".to_owned(), 0, 38));
        assert_eq!(k[2], ("function:A.f".to_owned(), 14, 36));
        assert!(k[3].0.starts_with("statement:"), "{k:?}");
        assert_eq!(k.len(), 4);
    }

    /// Grouped `var (...)` is `var_declaration > var_spec_list > var_spec`.
    #[test]
    fn grouped_go_vars_are_variables() {
        let src = "package p\n\nvar (\n\ta = 1\n\tb = 2\n)\n\nvar (\n\tc = 3\n)\n";
        let k = keys(src, Class::Go);
        let decls: Vec<_> = k
            .iter()
            .filter(|(k, ..)| k.starts_with("variable:"))
            .collect();
        assert_eq!(
            decls,
            [
                &("variable:a".to_owned(), 18, 23),
                &("variable:b".to_owned(), 25, 30),
                &("variable:c".to_owned(), 34, 48),
            ]
        );
    }

    #[test]
    fn class_by_extension() {
        for (path, class) in [
            ("a/README.MD", Class::Markdown),
            ("x.mdx", Class::Markdown),
            ("notes.txt", Class::Prose),
            ("a.d.ts", Class::Typescript),
            ("a.cjs", Class::Tsx),
            ("a.pyi", Class::Python),
            ("lib.rs", Class::Rust),
            ("Main.java", Class::Java),
            ("Makefile", Class::Text),
            ("dir.md/file", Class::Text),
            (".md", Class::Text),
        ] {
            assert_eq!(Class::from_path(path), class, "{path}");
        }
    }

    #[test]
    fn comments_are_found_iteratively() {
        let f = code("// a\nfn f() { /* b */ }\n/// c\nstruct S;", Class::Rust);
        assert_eq!(f.comments, [(0, 4), (14, 21), (24, 30)]);
        assert!(f.in_comment(15, 16));
        assert!(!f.in_comment(4, 5));
        assert_eq!(f.first_comment_start(5, 24), Some(14));
    }

    /// The work of `analyze_at_most` with a limit of `max` units.
    fn capped_work(text: &str, class: Class, max: usize) -> (bool, usize) {
        WORK.with(|w| w.set(0));
        let over = analyze_at_most(text, class, max).is_none();
        (over, WORK.with(|w| w.get()))
    }

    #[test]
    fn building_stops_as_soon_as_the_unit_count_passes_the_limit() {
        // Ruling on section 6.1: an over-limit resource is refused without
        // building its units. The work counter (root children counted,
        // discovery steps, lines scanned, tokens hashed) is bounded by the
        // limit, not by the input: doubling the input leaves it unchanged.
        const MAX: usize = 10;
        type Make = Box<dyn Fn(usize) -> String>;
        let inputs: [(&str, Make, Class); 3] = [
            (
                "top-level statements",
                Box::new(|n| "f();\n".repeat(n)),
                Class::Typescript,
            ),
            (
                "top-level declarations with comments",
                Box::new(|n| "function f(){} // c\n".repeat(n)),
                Class::Typescript,
            ),
            (
                "members of one class",
                Box::new(|n| format!("class C {{\n{}}}\n", "m() {}\n".repeat(n))),
                Class::Typescript,
            ),
        ];
        for (what, make, class) in &inputs {
            let (over_small, small) = capped_work(&make(1000), *class, MAX);
            let (over_large, large) = capped_work(&make(2000), *class, MAX);
            assert!(over_small && over_large, "{what}: not refused");
            assert_eq!(small, large, "{what}: work grew with the input");
            assert!(
                small <= 4 * MAX,
                "{what}: work {small} for a limit of {MAX}"
            );
            // Under the limit the same input is built in full.
            assert!(
                analyze_at_most(&make(3), *class, MAX).is_some(),
                "{what}: refused under the limit"
            );
        }
        // The limit counts the `file` unit: exactly `max` units is allowed.
        let text = "f();\n".repeat(MAX - 1);
        assert!(analyze_at_most(&text, Class::Typescript, MAX).is_some());
        assert!(analyze_at_most(&text, Class::Typescript, MAX - 1).is_none());
        let blocks = "x\n\n".repeat(MAX - 1);
        assert_eq!(
            text_units_at_most(&blocks, MAX - 1).map(|b| b.len()),
            Some(MAX - 1)
        );
        assert!(text_units_at_most(&blocks, MAX - 2).is_none());
    }

    #[test]
    fn first_error_line_names_the_first_error() {
        assert_eq!(
            first_error_line("x = 1\n\ndef f(:\n  pass\n", Class::Python),
            Some(3)
        );
        assert_eq!(first_error_line("fn a() {}\n", Class::Rust), None);
        assert_eq!(first_error_line("fn a() {\n", Class::Rust), Some(1));
        assert_eq!(first_error_line("anything", Class::Text), None);
    }
}
