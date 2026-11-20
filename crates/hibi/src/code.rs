//! Language table, parsing, declaration units and the semantic stream (spec
//! sections 6.1, 6.4, 6.5, 8.4, and `smallest_named_node` of section 16.1).
//!
//! The stream and unit functions are pure: no file, clock, environment or
//! process access. Every walk over a parse tree is iterative (section 6.1,
//! requirement R49): its stack use does not grow with the depth of the tree.
//! Offsets are UTF-8 bytes into the resource text of section 4.2.

use std::collections::HashMap;

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

fn node_text<'t>(node: Node<'_>, text: &'t str) -> &'t str {
    &text[node.byte_range()]
}

/// The `ast-v1` semantic stream of `node` (section 8.4): a pre-order walk
/// over all children, named and anonymous, in source order, skipping
/// comment subtrees and a comma directly before a closing delimiter.
/// `text` is the source the tree was parsed from.
pub fn semantic_stream(node: Node<'_>, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    let mut stack = vec![node];
    let mut kept = Vec::new();
    while let Some(n) = stack.pop() {
        if is_comment_kind(n.kind()) {
            continue;
        }
        let kind = n.kind();
        if n.child_count() == 0 {
            out.push(format!("{kind}:{}", node_text(n, text)));
        } else {
            out.push(kind.to_owned());
        }
        if is_content_literal_kind(kind) {
            out.push(format!("={}", text::normalize(node_text(n, text))));
        }
        // Children from last to first, so that the stack pops them in
        // source order; comments and trailing commas are dropped here.
        kept.clear();
        let mut next: Option<Node<'_>> = None;
        let children: Vec<Node<'_>> = n.children(&mut cursor).collect();
        for &c in children.iter().rev() {
            if is_comment_kind(c.kind()) {
                continue;
            }
            let trailing_comma = !c.is_named()
                && node_text(c, text) == ","
                && next.is_some_and(|x| {
                    !x.is_named() && matches!(node_text(x, text), ")" | "]" | "}" | ">")
                });
            next = Some(c);
            if !trailing_comma {
                kept.push(c);
            }
        }
        stack.extend(kept.iter().copied());
    }
    out
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
        (Class::Typescript | Class::Tsx, "enum_declaration") => decl("enum", Field, &[]),
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
        (Class::Rust, "struct_item" | "union_item") => decl("struct", Field, &[]),
        (Class::Rust, "enum_item") => decl("enum", Field, &[]),
        (Class::Rust, "trait_item") => decl("trait", Field, &["declaration_list"]),
        (Class::Rust, "impl_item") => decl("impl", RustImpl, &["declaration_list"]),
        (Class::Rust, "mod_item") => decl("module", RustMod, &["declaration_list"]),
        (Class::Rust, "type_item" | "associated_type") => decl("type", Field, &[]),
        (Class::Rust, "const_item" | "static_item") => decl("variable", Field, &[]),
        (Class::Rust, "macro_definition") => decl("macro", Field, &[]),

        (Class::Go, "function_declaration") => decl("function", Field, &[]),
        (Class::Go, "method_declaration") => decl("method", GoMethod, &[]),
        (Class::Go, "type_spec" | "type_alias") => decl("type", Field, &[]),
        (Class::Go, "const_spec" | "var_spec") => decl("variable", Field, &[]),

        (Class::Java, "class_declaration" | "record_declaration") => {
            decl("class", Field, &["class_body"])
        }
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
        _ => return None,
    })
}

fn without_ws(s: &str) -> String {
    s.chars().filter(|&c| !text::is_ws(c)).collect()
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
            let ty = without_ws(node_text(field("type")?, text));
            let t = ty.split('<').next().unwrap_or("").to_owned();
            let name = match field("trait") {
                Some(tr) => format!("impl {} for {t}", without_ws(node_text(tr, text))),
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
            let ty = node_text(param.child_by_field_name("type")?, text);
            let ty = ty.strip_prefix('*').unwrap_or(ty);
            let r = ty.split('[').next().unwrap_or("");
            own(format!("{r}.{}", node_text(field("name")?, text)))
        }
        NameRule::JavaDeclarator => {
            parent.filter(|p| matches!(p.kind(), "field_declaration" | "constant_declaration"))?;
            own(node_text(field("name")?, text).to_owned())
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
    /// The parse tree has an error: the file is class `text` for this read,
    /// with `prose-v1` block units. The `file` unit is not included; its
    /// fingerprint is `prose-v1` of the whole text.
    Text(Vec<TextUnit>),
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
}

/// A container body to visit.
struct Body<'t> {
    node: Node<'t>,
    prefix: Option<String>,
    container: Option<usize>,
    /// The root named child this body lies in; `None` for the root itself.
    top: Option<usize>,
}

/// Parse a file of a code class and build its units, or fall back to class
/// `text` when the tree has an error. For a class without a grammar the
/// result is the `text` fallback as well.
pub fn analyze(text: &str, class: Class) -> Analysis {
    let Some(tree) = parse(text, class) else {
        return Analysis::Text(text_units(text));
    };
    let root = tree.root_node();
    if root.has_error() {
        return Analysis::Text(text_units(text));
    }
    Analysis::Code(code_units(root, text, class))
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

fn code_units(root: Node<'_>, text: &str, class: Class) -> CodeFile {
    let found = discover(root, text, class);

    // Statements: named root children that are not comments, not unit
    // nodes, and contain no unit node.
    let mut has_unit = vec![false; root.named_child_count()];
    for f in &found {
        has_unit[f.top] = true;
    }
    let mut cursor = root.walk();
    let statements: Vec<Node<'_>> = root
        .named_children(&mut cursor)
        .enumerate()
        .filter(|&(i, c)| !has_unit[i] && !is_comment_kind(c.kind()))
        .map(|(_, c)| c)
        .collect();

    // Every unit with its unit node, then source order.
    enum Src {
        File,
        Decl(usize),
        Statement,
    }
    let mut all: Vec<(Node<'_>, Src)> = Vec::with_capacity(1 + found.len() + statements.len());
    all.push((root, Src::File));
    all.extend(
        found
            .iter()
            .enumerate()
            .map(|(i, f)| (f.unit_node, Src::Decl(i))),
    );
    all.extend(statements.into_iter().map(|n| (n, Src::Statement)));
    all[1..].sort_by_key(|(n, _)| (n.start_byte(), std::cmp::Reverse(n.end_byte())));

    let mut decl_pos = vec![0; found.len()];
    let mut units: Vec<CodeUnit> = Vec::with_capacity(all.len());
    for (node, src) in &all {
        let fp = ast_fingerprint(*node, text, class);
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
        let (line, end_line) = line_range(*node);
        units.push(CodeUnit {
            key,
            kind,
            symbol,
            parent: None,
            start: node.start_byte(),
            end: node.end_byte(),
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
    for (pos, (_, src)) in all.iter().enumerate().skip(1) {
        let parent = match src {
            Src::Decl(i) => found[*i].parent.map(|p| units[decl_pos[p]].key.clone()),
            _ => None,
        };
        units[pos].parent = Some(parent.unwrap_or_else(|| "file".to_owned()));
    }
    CodeFile {
        class,
        units,
        comments: comment_spans(root),
    }
}

/// Discovery of section 6.4, iterative. Declarations are returned in
/// pre-order (source order of their declaration nodes).
fn discover<'t>(root: Node<'t>, text: &str, class: Class) -> Vec<Found<'t>> {
    let wrappers = wrappers(class);
    let inner = inner_bodies(class);
    let mut found: Vec<Found<'t>> = Vec::new();
    let mut bodies = vec![Body {
        node: root,
        prefix: None,
        container: None,
        top: None,
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
        while let Some((node, chain, top)) = pending.pop() {
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
            let unit_kind = if d.unit_kind == "function"
                && matches!(
                    container_kind,
                    Some("class" | "interface" | "trait" | "impl")
                ) {
                "method"
            } else {
                d.unit_kind
            };
            let index = found.len();
            found.push(Found {
                node,
                unit_node: node,
                kind: unit_kind,
                symbol: join(&name),
                parent: body.container,
                top,
            });
            members.push((index, chain));
            if !d.bodies.is_empty() {
                let mut c = node.walk();
                let inner_body = node
                    .named_children(&mut c)
                    .find(|ch| d.bodies.contains(&ch.kind()));
                if let Some(b) = inner_body {
                    nested.push(Body {
                        node: b,
                        prefix: Some(join(&member_prefix)),
                        container: Some(index),
                        top: Some(top),
                    });
                }
            }
        }
        // Unit nodes: climb through a wrapper while this is the only
        // declaration discovered through it.
        let mut through: HashMap<usize, usize> = HashMap::new();
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
    order
        .iter()
        .map(|&old| {
            let mut f = slots[old].take().expect("each index once");
            f.parent = f.parent.map(|p| new_index[p]);
            f
        })
        .collect()
}

/// The `block` units of section 6.5 for class `text`: each maximal run of
/// lines that are not empty after trimming, from the first non-WS character
/// to the last, with `prose-v1` fingerprints and numbered keys. The `file`
/// unit is not included.
pub fn text_units(text: &str) -> Vec<TextUnit> {
    let mut spans: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut run: Option<(usize, usize, usize, usize)> = None;
    let mut start = 0;
    for (n, line) in text.split('\n').enumerate() {
        let lineno = n + 1;
        let t = text::trim(line);
        if t.is_empty() {
            spans.extend(run.take());
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
    let fps: Vec<Fingerprint> = spans
        .iter()
        .map(|&(s, e, _, _)| fingerprint::prose_v1(&text[s..e]))
        .collect();
    let mut keys: Vec<String> = fps
        .iter()
        .map(|fp| fingerprint::content_key("block", fp))
        .collect();
    fingerprint::number_duplicates(&mut keys);
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(text: &str, class: Class) -> CodeFile {
        match analyze(text, class) {
            Analysis::Code(f) => f,
            Analysis::Text(_) => panic!("{text:?} did not parse as {class:?}"),
        }
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
                        Analysis::Text(units) => assert_eq!(units.len(), 1, "{class:?}"),
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

    #[test]
    fn parse_error_falls_back_to_text_blocks() {
        let Analysis::Text(units) = analyze("def f(:\n    pass\n\n\n  x = 1  \n", Class::Python)
        else {
            panic!("expected the text fallback");
        };
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
}
