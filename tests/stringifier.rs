//! Port of the upstream `test/stringifier.test.js`.
//!
//! These cases cover style inference on hand-built and edited trees, which the
//! parsed-fixture suite never exercises.
//!
//! Two upstream cases are not portable: both subclass `Stringifier` to override
//! `raw()` / `rule()`. Rust has no inheritance, and a custom syntax here would
//! implement its own writer over the same [`postcss::Build`] trait instead.

use postcss::{parse, NewNode, RawValue, Tree};

#[test]
fn creates_trimmed_raw_property() {
    let mut tree = Tree::new();
    let decl = tree.create(NewNode::decl("color", "trim"));
    tree.raws_mut(decl).value = Some(RawValue {
        raw: "raw".into(),
        value: "trim".into(),
    });
    assert_eq!(tree.raw_value(decl, "value"), "raw");

    tree.set_value(decl, "trim1");
    assert_eq!(tree.raw_value(decl, "value"), "trim1");
}

#[test]
fn works_without_raw_value_magic() {
    let mut tree = Tree::new();
    let decl = tree.create(NewNode::decl("color", "1"));
    assert_eq!(tree.raw_value(decl, "value"), "1");
}

#[test]
fn uses_node_raw() {
    let mut tree = Tree::new();
    let rule = tree.create(NewNode::rule("a").between("\n"));
    assert_eq!(tree.raw(rule, Some("between"), Some("beforeOpen")), "\n");
}

#[test]
fn hacks_before_for_nodes_without_parent() {
    let mut tree = Tree::new();
    let rule = tree.create(NewNode::rule("a"));
    assert_eq!(tree.raw(rule, Some("before"), None), "");
}

#[test]
fn hacks_before_for_first_node() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::rule("a")).unwrap();
    let first = tree.first(root).unwrap();
    assert_eq!(tree.raw(first, Some("before"), None), "");
}

#[test]
fn hacks_before_for_first_decl() {
    let mut tree = Tree::new();
    let decl = tree.create(NewNode::decl("color", "black"));
    assert_eq!(tree.raw(decl, Some("before"), None), "");

    let rule = tree.create(NewNode::rule("a"));
    tree.append(rule, decl).unwrap();
    assert_eq!(tree.raw(decl, Some("before"), None), "\n    ");
}

#[test]
fn detects_after_raw() {
    let mut tree = Tree::new();
    let root = tree.root();
    let mut first = NewNode::rule("a");
    first.raws.after = Some(" ".into());
    tree.append(root, first).unwrap();
    let first = tree.first(root).unwrap();
    tree.append(first, NewNode::decl("color", "black")).unwrap();
    tree.append(root, NewNode::rule("a")).unwrap();

    let last = tree.last(root).unwrap();
    assert_eq!(tree.raw(last, Some("after"), None), " ");
}

#[test]
fn uses_defaults_without_parent() {
    let mut tree = Tree::new();
    let rule = tree.create(NewNode::rule("a"));
    assert_eq!(tree.raw(rule, Some("between"), Some("beforeOpen")), " ");
}

#[test]
fn uses_defaults_for_unique_node() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::rule("a")).unwrap();
    let first = tree.first(root).unwrap();
    assert_eq!(tree.raw(first, Some("between"), Some("beforeOpen")), " ");
}

#[test]
fn clones_raw_from_first_node() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::rule("a").between("")).unwrap();
    tree.append(root, NewNode::rule("b")).unwrap();

    let last = tree.last(root).unwrap();
    assert_eq!(tree.raw(last, Some("between"), Some("beforeOpen")), "");
}

#[test]
fn indents_by_default() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::at_rule("page", "")).unwrap();
    let page = tree.first(root).unwrap();
    tree.append(page, NewNode::rule("a")).unwrap();
    let rule = tree.first(page).unwrap();
    tree.append(rule, NewNode::decl("color", "black")).unwrap();

    assert_eq!(
        tree.to_css(),
        "@page {\n    a {\n        color: black\n    }\n}"
    );
}

#[test]
fn clones_style() {
    let mut compress = parse("@page{ a{ } }").unwrap();
    let page = compress.first(compress.root()).unwrap();
    let rule = compress.first(page).unwrap();
    compress
        .append(rule, NewNode::decl("color", "black"))
        .unwrap();
    assert_eq!(compress.to_css(), "@page{ a{ color: black } }");

    let mut spaces = parse("@page {\n  a {\n  }\n}").unwrap();
    let page = spaces.first(spaces.root()).unwrap();
    let rule = spaces.first(page).unwrap();
    spaces
        .append(rule, NewNode::decl("color", "black"))
        .unwrap();
    assert_eq!(spaces.to_css(), "@page {\n  a {\n    color: black\n  }\n}");
}

#[test]
fn clones_indent() {
    let mut tree = parse("a{\n}").unwrap();
    let rule = tree.first(tree.root()).unwrap();
    tree.append(rule, NewNode::comment("a")).unwrap();
    tree.append(rule, NewNode::comment("b").before("\n\n "))
        .unwrap();
    assert_eq!(tree.to_css(), "a{\n\n /* a */\n\n /* b */\n}");
}

#[test]
fn clones_declaration_before_for_comment() {
    let mut tree = parse("a{\n}").unwrap();
    let rule = tree.first(tree.root()).unwrap();
    tree.append(rule, NewNode::comment("a")).unwrap();
    tree.append(rule, NewNode::decl("a", "1").before("\n\n "))
        .unwrap();
    assert_eq!(tree.to_css(), "a{\n\n /* a */\n\n a: 1\n}");
}

#[test]
fn clones_indent_by_types() {
    let mut tree = parse("a {\n  *color: black\n}\n\nb {\n}").unwrap();
    let root = tree.root();
    tree.append(root, NewNode::rule("em")).unwrap();
    let last = tree.last(root).unwrap();
    tree.append(last, NewNode::decl("z-index", "1")).unwrap();

    let decl = tree.first(last).unwrap();
    assert_eq!(tree.raw(decl, Some("before"), None), "\n  ");
}

#[test]
fn ignores_non_space_symbols_in_indent_cloning() {
    let mut tree = parse("a {\n  color: black\n}\n\nb {\n}").unwrap();
    let root = tree.root();
    tree.append(root, NewNode::rule("em")).unwrap();
    let last = tree.last(root).unwrap();
    tree.append(last, NewNode::decl("z-index", "1")).unwrap();

    assert_eq!(tree.raw(last, Some("before"), None), "\n\n");
    let decl = tree.first(last).unwrap();
    assert_eq!(tree.raw(decl, Some("before"), None), "\n  ");
}

#[test]
fn clones_indent_by_before_and_after() {
    let mut tree = parse("@page{\n\n a{\n  color: black}}").unwrap();
    let page = tree.first(tree.root()).unwrap();
    tree.append(page, NewNode::rule("b")).unwrap();
    let last = tree.last(page).unwrap();
    tree.append(last, NewNode::decl("z-index", "1")).unwrap();

    assert_eq!(tree.raw(last, Some("before"), None), "\n\n ");
    assert_eq!(tree.raw(last, Some("after"), None), "");
}

#[test]
fn terminates_childless_at_rule_followed_by_a_comment() {
    let mut tree = parse("a {}\n/* comment */").unwrap();
    let last = tree.last(tree.root()).unwrap();
    tree.insert_before(last, NewNode::at_rule("import", "\"x.css\""))
        .unwrap();

    assert_eq!(tree.to_css(), "a {}\n@import \"x.css\";\n/* comment */");

    let reparsed = parse(tree.to_css()).unwrap();
    let types: Vec<&str> = reparsed
        .children(reparsed.root())
        .iter()
        .map(|&node| reparsed.type_name(node))
        .collect();
    assert_eq!(types, ["rule", "atrule", "comment"]);
}

#[test]
fn terminates_nested_childless_at_rule_followed_by_a_comment() {
    let mut tree = parse("@media screen {\n  a {}\n}").unwrap();
    let media = tree.first(tree.root()).unwrap();
    tree.append(media, NewNode::at_rule("import", "\"y.css\""))
        .unwrap();
    tree.append(media, NewNode::comment("note")).unwrap();

    assert_eq!(
        tree.to_css(),
        "@media screen {\n  a {}\n  @import \"y.css\";\n  /* note */\n}"
    );

    let reparsed = parse(tree.to_css()).unwrap();
    let media = reparsed.first(reparsed.root()).unwrap();
    let types: Vec<&str> = reparsed
        .children(media)
        .iter()
        .map(|&node| reparsed.type_name(node))
        .collect();
    assert_eq!(types, ["rule", "atrule", "comment"]);
}

#[test]
fn terminates_custom_property_followed_by_a_comment() {
    let mut tree = parse("a{--x:red}").unwrap();
    let rule = tree.first(tree.root()).unwrap();
    tree.append(rule, NewNode::comment("note")).unwrap();

    assert_eq!(tree.to_css(), "a{--x:red;/* note */}");

    let reparsed = parse(tree.to_css()).unwrap();
    let rule = reparsed.first(reparsed.root()).unwrap();
    let types: Vec<&str> = reparsed
        .children(rule)
        .iter()
        .map(|&node| reparsed.type_name(node))
        .collect();
    assert_eq!(types, ["decl", "comment"]);
}

#[test]
fn terminates_custom_property_with_important_before_a_comment() {
    let mut tree = parse("a{--x:red !important}").unwrap();
    let rule = tree.first(tree.root()).unwrap();
    let decl = tree.first(rule).unwrap();
    tree.insert_after(decl, NewNode::comment("note")).unwrap();

    assert_eq!(tree.to_css(), "a{--x:red !important;/* note */}");

    let reparsed = parse(tree.to_css()).unwrap();
    let rule = reparsed.first(reparsed.root()).unwrap();
    let types: Vec<&str> = reparsed
        .children(rule)
        .iter()
        .map(|&node| reparsed.type_name(node))
        .collect();
    assert_eq!(types, ["decl", "comment"]);
}

#[test]
fn clones_only_spaces_in_before() {
    let mut tree = parse("a{*one:1}").unwrap();
    let root = tree.root();
    let rule = tree.first(root).unwrap();
    tree.append(rule, NewNode::decl("two", "2")).unwrap();
    tree.append(root, NewNode::at_rule("keyframes", "a"))
        .unwrap();
    let last = tree.last(root).unwrap();
    tree.append(last, NewNode::rule("from")).unwrap();

    assert_eq!(tree.to_css(), "a{*one:1;two:2}\n@keyframes a{\nfrom{}}");
}

#[test]
fn clones_only_spaces_in_between() {
    let mut tree = parse("a{one/**/:1}").unwrap();
    let rule = tree.first(tree.root()).unwrap();
    tree.append(rule, NewNode::decl("two", "2")).unwrap();
    assert_eq!(tree.to_css(), "a{one/**/:1;two:2}");
}

#[test]
fn uses_optional_raws_indent() {
    let mut tree = Tree::new();
    let mut rule = NewNode::rule("a");
    rule.raws.indent = Some(" ".into());
    let rule = tree.create(rule);
    tree.append(rule, NewNode::decl("color", "black")).unwrap();
    assert_eq!(tree.node_to_css(rule), "a {\n color: black\n}");
}

#[test]
fn handles_nested_roots() {
    let mut tree = Tree::new();
    let root = tree.root();

    let mut sub = Tree::new();
    let sub_root = sub.root();
    sub.append(sub_root, NewNode::at_rule("foo", "")).unwrap();

    tree.append(root, sub).unwrap();
    assert_eq!(tree.to_css(), "@foo");
}

#[test]
fn handles_root() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::at_rule("foo", "")).unwrap();
    assert_eq!(tree.to_css(), "@foo");
}

#[test]
fn handles_root_with_after() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.raws_mut(root).after = Some("   ".into());
    tree.append(root, NewNode::at_rule("foo", "")).unwrap();
    assert_eq!(tree.to_css(), "@foo   ");
}

#[test]
fn passes_nodes_to_document() {
    let mut document = Tree::new_document();
    let root = document.root();
    document.append(root, NewNode::root()).unwrap();
    assert_eq!(document.to_css(), "");
}

#[test]
fn handles_document_with_one_root() {
    let mut root = Tree::new();
    let root_id = root.root();
    root.append(root_id, NewNode::at_rule("foo", "")).unwrap();

    let mut document = Tree::new_document();
    let document_id = document.root();
    document.append(document_id, root).unwrap();

    assert_eq!(document.to_css(), "@foo");
}

#[test]
fn handles_document_with_one_root_and_after_raw() {
    let mut root = Tree::new();
    let root_id = root.root();
    root.raws_mut(root_id).after = Some("   ".into());
    root.append(root_id, NewNode::at_rule("foo", "")).unwrap();

    let mut document = Tree::new_document();
    let document_id = document.root();
    document.append(document_id, root).unwrap();

    assert_eq!(document.to_css(), "@foo   ");
}

#[test]
fn handles_document_with_three_roots_without_raws() {
    let mut document = Tree::new_document();
    let document_id = document.root();

    let mut first = Tree::new();
    let id = first.root();
    first.append(id, NewNode::at_rule("foo", "")).unwrap();

    let mut second = Tree::new();
    let id = second.root();
    second.append(id, NewNode::rule("a")).unwrap();

    let mut third = Tree::new();
    let id = third.root();
    third.append(id, NewNode::decl("color", "black")).unwrap();

    document.append(document_id, first).unwrap();
    document.append(document_id, second).unwrap();
    document.append(document_id, third).unwrap();

    assert_eq!(document.to_css(), "@fooa {}color: black");
}

#[test]
fn handles_document_with_three_roots_with_before_and_after_raws() {
    let mut document = Tree::new_document();
    let document_id = document.root();

    for (selector, after) in [
        ("a.one", "AFTER_ONE"),
        ("a.two", "AFTER_TWO"),
        ("a.three", "AFTER_THREE"),
    ] {
        let mut root = Tree::new();
        let id = root.root();
        root.raws_mut(id).after = Some(after.into());
        root.append(id, NewNode::rule(selector)).unwrap();
        document.append(document_id, root).unwrap();
    }

    assert_eq!(
        document.to_css(),
        "a.one {}AFTER_ONEa.two {}AFTER_TWOa.three {}AFTER_THREE"
    );
}

#[test]
fn escapes_style_and_comment_open_with_css_escape() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::rule("</style>")).unwrap();
    tree.append(root, NewNode::at_rule("media", "<style>"))
        .unwrap();
    tree.append(root, NewNode::comment("</style><!--<style>"))
        .unwrap();

    let mut rule = NewNode::rule("a");
    rule.raws.before = Some("\n</style>".into());
    rule.raws.after = Some("</style>".into());
    let rule = tree.create(rule);
    tree.append(rule, NewNode::decl("color", "</style>"))
        .unwrap();
    tree.append(root, rule).unwrap();

    assert_eq!(
        tree.to_css(),
        concat!(
            "\\3c /style> {}\n",
            "@media \\3c style>;\n",
            "/* \\3c /style>\\3c !--\\3c style> */\n",
            "\\3c /style>a {\n",
            "    color: \\3c /style>",
            "\\3c /style>}"
        )
    );
}

#[test]
fn does_not_escape_document_raws() {
    let mut document = Tree::new_document();
    let document_id = document.root();

    let mut first = Tree::new();
    let id = first.root();
    first.append(id, NewNode::rule("a")).unwrap();

    let mut second = Tree::new();
    let id = second.root();
    second.raws_mut(id).before = Some("</style>".into());
    second.raws_mut(id).after = Some("</style>".into());
    second.append(id, NewNode::rule("b")).unwrap();

    document.append(document_id, first).unwrap();
    document.append(document_id, second).unwrap();

    assert_eq!(document.to_css(), "a {}</style>b {}</style>");
}

#[test]
fn adds_space_before_params_set_on_an_at_rule_parsed_without_them() {
    let mut tree = parse("@layer{a{color:black}}").unwrap();
    let layer = tree.first(tree.root()).unwrap();
    tree.set_params(layer, "utilities");
    assert_eq!(tree.to_css(), "@layer utilities{a{color:black}}");

    let mut tree = parse("@media;").unwrap();
    let media = tree.first(tree.root()).unwrap();
    tree.set_params(media, "print");
    assert_eq!(tree.node_to_css(media), "@media print");
}

#[test]
fn keeps_params_glued_to_at_rule_name_when_css_allows_it() {
    let mut tree = parse("@media(min-width:0){}").unwrap();
    let media = tree.first(tree.root()).unwrap();
    tree.set_params(media, "(min-width:1px)");
    assert_eq!(tree.to_css(), "@media(min-width:1px){}");

    let mut tree = parse("@import\"a.css\"").unwrap();
    let imported = tree.first(tree.root()).unwrap();
    tree.set_params(imported, "\"b.css\"");
    assert_eq!(tree.node_to_css(imported), "@import\"b.css\"");
}

#[test]
fn clones_semicolon_only_from_rules_with_children() {
    let tree = parse("a{}b{one:1;}").unwrap();
    let first = tree.first(tree.root()).unwrap();
    // `raws.semicolon` is inferred from the rule that has declarations.
    assert!(tree.node(first).raws.semicolon.is_none());
    assert_eq!(tree.to_css(), "a{}b{one:1;}");
}
