//! Port of the upstream `test/container.test.ts` and `test/node.test.ts`.
//!
//! Cases that only exercise JavaScript dynamics — assigning arbitrary
//! properties to a node, subclassing, coercing a number into a value string —
//! have no counterpart in a typed API and are left out.

use std::cmp::Ordering;

use postcss::{parse, NewNode, NodeErrorOptions, Tree, Visit};

fn first_rule(tree: &Tree) -> postcss::NodeId {
    tree.first(tree.root()).expect("a first child")
}

fn props(tree: &Tree, parent: postcss::NodeId) -> Vec<String> {
    tree.children(parent)
        .iter()
        .filter_map(|&node| tree.prop(node).map(str::to_string))
        .collect()
}

// --- push / each ---------------------------------------------------------

#[test]
fn push_adds_child_without_checks() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.create(NewNode::decl("c", "3"));
    tree.push_child_public(rule, decl);

    assert_eq!(tree.node_to_css(rule), "a { a: 1; b: 2; c: 3 }");
    assert_eq!(tree.children(rule).len(), 3);
    assert!(tree.raws(decl).before.is_none());
}

#[test]
fn each_iterates() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut indexes = Vec::new();
    let completed = tree.each(rule, |tree, node, index| {
        indexes.push(index);
        assert_eq!(tree.children(rule)[index], node);
    });

    assert!(completed);
    assert_eq!(indexes, [0, 1]);
}

#[test]
fn each_iterates_with_prepend() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut size = 0;
    tree.each(rule, |tree, _, _| {
        tree.prepend(rule, NewNode::decl("color", "aqua")).unwrap();
        size += 1;
    });

    assert_eq!(size, 2);
}

#[test]
fn each_iterates_with_prepend_insert_before() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut size = 0;
    tree.each(rule, |tree, node, _| {
        if tree.prop(node) == Some("a") {
            tree.insert_before(node, NewNode::decl("c", "3")).unwrap();
        }
        size += 1;
    });

    assert_eq!(size, 2);
}

#[test]
fn each_iterates_with_append_insert_before() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut size = 0;
    tree.each(rule, |tree, node, index| {
        if tree.prop(node) == Some("a") {
            tree.insert_before_index(rule, index + 1, NewNode::decl("c", "3"))
                .unwrap();
        }
        size += 1;
    });

    assert_eq!(size, 3);
}

#[test]
fn each_iterates_with_prepend_insert_after() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    // Upstream calls `insertAfter(i - 1, …)`, which for `i === 0` lands before
    // every cursor and shifts them all — the same effect as prepending.
    let mut size = 0;
    tree.each(rule, |tree, _, index| {
        if index == 0 {
            tree.prepend(rule, NewNode::decl("c", "3")).unwrap();
        } else {
            tree.insert_after_index(rule, index - 1, NewNode::decl("c", "3"))
                .unwrap();
        }
        size += 1;
    });

    assert_eq!(size, 2);
}

#[test]
fn each_iterates_with_append_insert_after() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut size = 0;
    tree.each(rule, |tree, node, index| {
        if tree.prop(node) == Some("a") {
            tree.insert_after_index(rule, index, NewNode::decl("c", "3"))
                .unwrap();
        }
        size += 1;
    });

    assert_eq!(size, 3);
}

#[test]
fn each_iterates_with_remove() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut size = 0;
    tree.each(rule, |tree, _, _| {
        tree.remove_child_at(rule, 0);
        size += 1;
    });

    assert_eq!(size, 2);
}

#[test]
fn each_breaks_iteration() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut indexes = Vec::new();
    let completed = tree.each(rule, |_, _, index| {
        indexes.push(index);
        Visit::Break
    });

    assert!(!completed);
    assert_eq!(indexes, [0]);
}

#[test]
fn each_allows_to_change_children() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);

    let mut collected = Vec::new();
    tree.each(rule, |tree, node, _| {
        collected.push(tree.prop(node).unwrap_or_default().to_string());
        let children = tree.children(rule).to_vec();
        for child in children {
            tree.remove(child);
        }
    });

    assert_eq!(collected, ["a"]);
}

// --- walk ----------------------------------------------------------------

#[test]
fn walk_iterates() {
    let mut tree = parse("a { b { c: 1 } }").unwrap();
    let mut types = Vec::new();
    tree.walk_all(|tree, node| {
        types.push(tree.type_name(node));
    });
    assert_eq!(types, ["rule", "rule", "decl"]);
}

#[test]
fn walk_breaks_iteration() {
    let mut tree = parse("a { b { c: 1 } }").unwrap();
    let mut count = 0;
    let completed = tree.walk_all(|_, _| {
        count += 1;
        Visit::Break
    });
    assert!(!completed);
    assert_eq!(count, 1);
}

#[test]
fn walk_decls_iterates() {
    let mut tree = parse("a { b: 1; c { d: 2 } }").unwrap();
    let mut found = Vec::new();
    tree.walk_decls(|tree, decl| {
        found.push(tree.prop(decl).unwrap_or_default().to_string());
    });
    assert_eq!(found, ["b", "d"]);
}

#[test]
fn walk_decls_iterates_with_changes() {
    let mut tree = parse("a { p1: 1; p2: 2 }").unwrap();
    let mut size = 0;
    tree.walk_decls(|tree, decl| {
        tree.remove(decl);
        size += 1;
    });
    assert_eq!(size, 2);
}

#[test]
fn walk_decls_filters_by_property_name() {
    let mut tree = parse("a { a: 1; b: 2 } c { a: 3 }").unwrap();
    let mut count = 0;
    tree.walk_decls_with_prop("a", |tree, decl| {
        assert_eq!(tree.prop(decl), Some("a"));
        count += 1;
    });
    assert_eq!(count, 2);
}

#[test]
fn walk_comments_iterates() {
    let mut tree = parse("/* a */ b { /* c */ }").unwrap();
    let mut texts = Vec::new();
    tree.walk_comments(|tree, comment| {
        texts.push(tree.text(comment).unwrap_or_default().to_string());
    });
    assert_eq!(texts, ["a", "c"]);
}

#[test]
fn walk_rules_iterates_and_filters() {
    let mut tree = parse("a {} b { c {} }").unwrap();
    let mut selectors = Vec::new();
    tree.walk_rules(|tree, rule| {
        selectors.push(tree.selector(rule).unwrap_or_default().to_string());
    });
    assert_eq!(selectors, ["a", "b", "c"]);

    let mut count = 0;
    tree.walk_rules_with_selector("c", |_, _| count += 1);
    assert_eq!(count, 1);
}

#[test]
fn walk_at_rules_iterates_and_filters() {
    let mut tree = parse("@media a { @page {} } @import b;").unwrap();
    let mut names = Vec::new();
    tree.walk_at_rules(|tree, at_rule| {
        names.push(tree.name(at_rule).unwrap_or_default().to_string());
    });
    assert_eq!(names, ["media", "page", "import"]);

    let mut count = 0;
    tree.walk_at_rules_with_name("page", |_, _| count += 1);
    assert_eq!(count, 1);
}

// --- append / prepend ----------------------------------------------------

#[test]
fn append_appends_child() {
    let mut tree = parse("a { a: 1 }").unwrap();
    let rule = first_rule(&tree);
    tree.append(rule, NewNode::decl("b", "2")).unwrap();

    assert_eq!(tree.to_css(), "a { a: 1; b: 2 }");
    let last = tree.last(rule).unwrap();
    assert_eq!(tree.raws(last).before.as_deref(), Some(" "));
}

#[test]
fn append_appends_multiple_children() {
    let mut tree = parse("a { a: 1 }").unwrap();
    let rule = first_rule(&tree);
    tree.append(rule, vec![NewNode::decl("b", "2"), NewNode::decl("c", "3")])
        .unwrap();

    assert_eq!(tree.to_css(), "a { a: 1; b: 2; c: 3 }");
}

#[test]
fn append_has_rule_at_rule_and_comment_shortcuts() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, NewNode::rule("a")).unwrap();
    tree.append(root, NewNode::at_rule("media", "screen"))
        .unwrap();
    tree.append(root, NewNode::comment("c")).unwrap();

    assert_eq!(tree.to_css(), "a {}\n@media screen;\n/* c */");
}

#[test]
fn append_receives_root() {
    let mut tree = parse("a {}").unwrap();
    let root = tree.root();
    let other = parse("b {}").unwrap();
    tree.append(root, other).unwrap();

    // The appended nodes keep the `before` they were parsed with, and the
    // sample is the root's first child, so nothing is rewritten.
    assert_eq!(tree.to_css(), "a {}b {}");
}

#[test]
fn prepend_receives_root() {
    let mut tree = parse("a {}").unwrap();
    let root = tree.root();
    let other = parse("b {}").unwrap();
    tree.prepend(root, other).unwrap();

    assert_eq!(tree.to_css(), "b {}\na {}");
}

#[test]
fn prepend_receives_string() {
    let mut tree = parse("a {}").unwrap();
    let root = tree.root();
    tree.prepend(root, "b {}").unwrap();

    assert_eq!(tree.to_css(), "b {}\na {}");
}

#[test]
fn append_receives_string() {
    let mut tree = Tree::new();
    let root = tree.root();
    tree.append(root, "a{}b{}").unwrap();
    let a = tree.first(root).unwrap();
    tree.append(a, "color:black").unwrap();

    assert_eq!(tree.to_css(), "a{color:black}b{}");
    // Parsed nodes lose their source, since it refers to another input.
    let decl = tree.first(a).unwrap();
    assert!(tree.source(decl).is_none());
}

#[test]
fn append_moves_node_on_insert() {
    let mut tree = parse("a{} b{}").unwrap();
    let root = tree.root();
    let a = tree.first(root).unwrap();
    let b = tree.last(root).unwrap();

    tree.append(b, a).unwrap();
    assert_eq!(tree.to_css(), "b{a{}}");
    assert_eq!(tree.children(root), [b]);
    assert_eq!(tree.parent(a), Some(b));
}

#[test]
fn prepend_prepends_child() {
    let mut tree = parse("a { a: 1 }").unwrap();
    let rule = first_rule(&tree);
    tree.prepend(rule, NewNode::decl("b", "2")).unwrap();

    assert_eq!(tree.to_css(), "a { b: 2; a: 1 }");
    let first = tree.first(rule).unwrap();
    assert_eq!(tree.raws(first).before.as_deref(), Some(" "));
}

#[test]
fn prepend_prepends_multiple_children() {
    let mut tree = parse("a { a: 1 }").unwrap();
    let rule = first_rule(&tree);
    tree.prepend(rule, vec![NewNode::decl("b", "2"), NewNode::decl("c", "3")])
        .unwrap();

    assert_eq!(tree.to_css(), "a { b: 2; c: 3; a: 1 }");
}

#[test]
fn prepend_works_on_empty_container() {
    let mut tree = parse("").unwrap();
    let root = tree.root();
    tree.prepend(root, "a {}").unwrap();
    assert_eq!(tree.to_css(), "a {}");
}

#[test]
fn prepend_moves_the_first_nodes_before_to_the_next() {
    let mut tree = parse("a {}\nb {}").unwrap();
    let root = tree.root();
    tree.prepend(root, NewNode::rule("em")).unwrap();
    assert_eq!(tree.to_css(), "em {}\na {}\nb {}");
}

// --- insertBefore / insertAfter -----------------------------------------

#[test]
fn insert_before_inserts_child() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    let b = tree.last(rule).unwrap();
    tree.insert_before(b, NewNode::decl("c", "3")).unwrap();

    assert_eq!(tree.to_css(), "a { a: 1; c: 3; b: 2 }");
}

#[test]
fn insert_before_receives_pre_existing_child_node() {
    let mut tree = parse("a { a: 1; b: 2; c: 3 }").unwrap();
    let rule = first_rule(&tree);
    let a = tree.first(rule).unwrap();
    let c = tree.last(rule).unwrap();

    tree.insert_before(a, c).unwrap();
    assert_eq!(props(&tree, rule), ["c", "a", "b"]);
}

#[test]
fn insert_after_inserts_child() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    let a = tree.first(rule).unwrap();
    tree.insert_after(a, NewNode::decl("c", "3")).unwrap();

    assert_eq!(tree.to_css(), "a { a: 1; c: 3; b: 2 }");
}

#[test]
fn insert_after_receives_pre_existing_child_node() {
    let mut tree = parse("a { a: 1; b: 2; c: 3 }").unwrap();
    let rule = first_rule(&tree);
    let a = tree.first(rule).unwrap();
    let c = tree.last(rule).unwrap();

    tree.insert_after(a, c).unwrap();
    assert_eq!(props(&tree, rule), ["a", "c", "b"]);
}

#[test]
fn insert_before_has_defined_way_of_adding_newlines() {
    let mut tree = parse("a {}").unwrap();
    let root = tree.root();
    let a = tree.first(root).unwrap();
    tree.insert_before(a, NewNode::rule("b")).unwrap();
    assert_eq!(tree.to_css(), "b {}\na {}");
}

// --- removal -------------------------------------------------------------

#[test]
fn remove_child_removes_by_index_and_node() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    tree.remove_child_at(rule, 0);
    assert_eq!(tree.node_to_css(rule), "a { b: 2 }");

    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    let b = tree.last(rule).unwrap();
    tree.remove_child(rule, b);
    assert_eq!(tree.node_to_css(rule), "a { a: 1 }");
}

#[test]
fn remove_child_cleans_parent_in_removed_node() {
    let mut tree = parse("a { a: 1 }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    tree.remove_child(rule, decl);
    assert_eq!(tree.parent(decl), None);
}

#[test]
fn remove_all_removes_all_children() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    tree.remove_all(rule);

    assert_eq!(tree.parent(decl), None);
    assert_eq!(tree.node_to_css(rule), "a { }");
}

#[test]
fn remove_node_from_parent() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    tree.remove(decl);

    assert_eq!(tree.to_css(), "a { b: 2 }");
    assert_eq!(tree.parent(decl), None);
}

// --- replaceValues / every / some / index -------------------------------

#[test]
fn replace_values_replaces_strings() {
    let mut tree = parse("a{one:1}b{two:1 2}").unwrap();
    tree.replace_values("1", None, "A");
    assert_eq!(tree.to_css(), "a{one:A}b{two:A 2}");
}

#[test]
fn replace_values_filters_properties() {
    let mut tree = parse("a{one:1}b{two:1 2}").unwrap();
    tree.replace_values("1", Some(&["one"]), "A");
    assert_eq!(tree.to_css(), "a{one:A}b{two:1 2}");
}

#[test]
fn every_and_some() {
    let tree = parse("a{one:1;two:2}").unwrap();
    let rule = first_rule(&tree);

    assert!(tree.every(rule, |tree, node| tree.prop(node).is_some()));
    assert!(!tree.every(rule, |tree, node| tree.prop(node) == Some("one")));
    assert!(tree.some(rule, |tree, node| tree.prop(node) == Some("one")));
    assert!(!tree.some(rule, |tree, node| tree.prop(node) == Some("three")));
}

#[test]
fn index_returns_child_index() {
    let tree = parse("a{one:1;two:2}").unwrap();
    let rule = first_rule(&tree);
    let second = tree.children(rule)[1];
    assert_eq!(tree.index(rule, second), Some(1));
}

#[test]
fn first_and_last_work_for_children_less_nodes() {
    let tree = parse("a{color:black}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    assert_eq!(tree.first(decl), None);
    assert_eq!(tree.last(decl), None);
    assert_eq!(tree.first(rule), Some(decl));
    assert_eq!(tree.last(rule), Some(decl));
}

#[test]
fn children_can_be_sorted() {
    let mut tree = parse("a{b:1;c:2;a:3}").unwrap();
    let rule = first_rule(&tree);
    tree.sort_children(rule, |tree, a, b| match (tree.prop(a), tree.prop(b)) {
        (Some(a), Some(b)) => a.cmp(b),
        _ => Ordering::Equal,
    });
    assert_eq!(props(&tree, rule), ["a", "b", "c"]);
}

#[test]
fn does_not_normalize_new_children_with_existing_before() {
    let mut tree = parse("a { a: 1; b: 2 }").unwrap();
    let rule = first_rule(&tree);
    tree.append(rule, NewNode::decl("c", "3").before("\n "))
        .unwrap();
    assert_eq!(tree.node_to_css(rule), "a { a: 1; b: 2;\n c: 3 }");
}

#[test]
fn keeps_an_explicit_before_when_appending_to_a_root() {
    let mut tree = parse("a {}\n\nb {}").unwrap();
    let root = tree.root();
    tree.append(root, NewNode::rule("c").before("\n\n\n"))
        .unwrap();
    let last = tree.last(root).unwrap();
    assert_eq!(tree.raws(last).before.as_deref(), Some("\n\n\n"));
}

#[test]
fn rewrites_before_when_appending_parsed_css_to_a_root() {
    let mut tree = parse("a {}\n\nb {}").unwrap();
    let root = tree.root();
    tree.append(root, "c {}").unwrap();
    let last = tree.last(root).unwrap();
    // Parsed nodes are not protected, so they take the sample's `before`.
    assert_eq!(tree.raws(last).before.as_deref(), Some("\n\n"));
}

// --- node methods --------------------------------------------------------

#[test]
fn replace_with_inserts_new_node() {
    let mut tree = parse("a{one:1;two:2}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    tree.replace_with(decl, NewNode::decl("three", "3"))
        .unwrap();

    assert_eq!(tree.to_css(), "a{three:3;two:2}");
    assert_eq!(tree.parent(decl), None);
}

#[test]
fn replace_with_replaces_node_with_several() {
    let mut tree = parse("a{one:1;two:2}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    tree.replace_with(
        decl,
        vec![NewNode::decl("three", "3"), NewNode::decl("four", "4")],
    )
    .unwrap();

    assert_eq!(tree.to_css(), "a{three:3;four:4;two:2}");
}

#[test]
fn replace_with_can_include_itself() {
    let mut tree = parse("a{one:1;two:2}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    tree.replace_with(
        decl,
        vec![
            postcss::Insertable::from(NewNode::decl("zero", "0")),
            postcss::Insertable::from(decl),
            postcss::Insertable::from(NewNode::decl("three", "3")),
        ],
    )
    .unwrap();

    assert_eq!(tree.to_css(), "a{zero:0;one:1;three:3;two:2}");
}

#[test]
fn clone_clones_nodes() {
    let mut tree = parse("a { color: black }").unwrap();
    let rule = first_rule(&tree);
    let cloned = tree.clone_node(rule);

    assert_eq!(tree.parent(cloned), None);
    // The clone is deep: its declaration is a separate node.
    let original_decl = tree.first(rule).unwrap();
    let cloned_decl = tree.first(cloned).unwrap();
    assert_ne!(original_decl, cloned_decl);
    assert_eq!(tree.node_to_css(cloned), "a { color: black }");

    tree.set_value(cloned_decl, "red");
    assert_eq!(tree.value(original_decl), Some("black"));
}

#[test]
fn clone_keeps_code_style() {
    let mut tree = parse("@page 1{a{color:black;}}").unwrap();
    let root = tree.root();
    let cloned = tree.clone_node(root);
    assert_eq!(tree.node_to_css(cloned), "@page 1{a{color:black;}}");
}

#[test]
fn clone_before_and_after() {
    let mut tree = parse("a { color: black }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    tree.clone_before(decl).unwrap();
    assert_eq!(tree.to_css(), "a { color: black; color: black }");

    let mut tree = parse("a { color: black }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    tree.clone_after(decl).unwrap();
    assert_eq!(tree.to_css(), "a { color: black; color: black }");
}

#[test]
fn next_and_prev() {
    let mut tree = parse("a{one:1;two:2}").unwrap();
    let rule = first_rule(&tree);
    let one = tree.first(rule).unwrap();
    let two = tree.last(rule).unwrap();

    assert_eq!(tree.next(one), Some(two));
    assert_eq!(tree.next(two), None);
    assert_eq!(tree.prev(two), Some(one));
    assert_eq!(tree.prev(one), None);

    let detached = tree.create(NewNode::decl("a", "1"));
    assert_eq!(tree.next(detached), None);
    assert_eq!(tree.prev(detached), None);
}

#[test]
fn root_returns_root() {
    let tree = parse("@media a { b { color: black } }").unwrap();
    let root = tree.root();
    let media = tree.first(root).unwrap();
    let rule = tree.first(media).unwrap();
    let decl = tree.first(rule).unwrap();

    assert_eq!(tree.root_of(decl), root);
    assert_eq!(tree.root_of(root), root);
}

#[test]
fn root_returns_root_inside_document() {
    let mut document = Tree::new_document();
    let document_id = document.root();
    let inner = parse("a { color: black }").unwrap();
    document.append(document_id, inner).unwrap();

    let root = document.first(document_id).unwrap();
    let rule = document.first(root).unwrap();
    // `root()` stops at the document rather than crossing it.
    assert_eq!(document.root_of(rule), root);
    assert_eq!(document.root_of(document_id), document_id);
}

#[test]
fn clean_raws_cleans_style_recursively() {
    let mut tree = parse("@page{a{color:black}}").unwrap();
    let root = tree.root();
    tree.clean_raws(root, false);
    assert_eq!(
        tree.to_css(),
        "@page {\n    a {\n        color: black\n    }\n}"
    );

    let mut tree = parse("@page{a{color:black}}").unwrap();
    let root = tree.root();
    tree.clean_raws(root, true);
    assert_eq!(
        tree.to_css(),
        "@page{\n    a{\n        color:black\n    }\n}"
    );
}

// --- positions -----------------------------------------------------------

fn at(position: postcss::Position) -> (usize, usize, usize) {
    (position.line, position.column, position.offset)
}

#[test]
fn position_inside_returns_position_when_node_starts_mid_line() {
    let tree = parse("a {  one: X  }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    assert_eq!(at(tree.position_inside(decl, 6)), (1, 12, 11));
}

#[test]
fn position_inside_returns_position_when_before_contains_newline() {
    let tree = parse("a {\n  one: X}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    assert_eq!(at(tree.position_inside(decl, 6)), (2, 9, 12));
}

#[test]
fn position_inside_returns_position_when_node_contains_newlines() {
    let tree = parse("a {\n\tone: 1\n\t\tX\n3}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    assert_eq!(at(tree.position_inside(decl, 10)), (3, 4, 15));
}

#[test]
fn position_by_returns_position() {
    let tree = parse("a {  one: X  }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    assert_eq!(
        at(tree.position_by(decl, &NodeErrorOptions::default())),
        (1, 6, 5)
    );
    assert_eq!(
        at(tree.position_by(rule, &NodeErrorOptions::default())),
        (1, 1, 0)
    );
}

#[test]
fn position_by_returns_position_for_word() {
    let tree = parse("a {  one: X  }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    let word = |node, word: &str| {
        at(tree.position_by(
            node,
            &NodeErrorOptions {
                word: Some(word.into()),
                ..Default::default()
            },
        ))
    };

    assert_eq!(word(decl, "one"), (1, 6, 5));
    assert_eq!(word(decl, "X"), (1, 11, 10));
    assert_eq!(word(rule, "}"), (1, 14, 13));
}

#[test]
fn position_by_returns_position_after_ast_mutations() {
    let mut tree = parse("a {\n\tone: 1;\n\ttwo: 2;}").unwrap();
    let rule = first_rule(&tree);
    let one = tree.first(rule).unwrap();
    let two = tree.next(one).unwrap();

    assert_eq!(
        at(tree.position_by(rule, &NodeErrorOptions::default())),
        (1, 1, 0)
    );
    assert_eq!(
        at(tree.position_by(two, &NodeErrorOptions::default())),
        (3, 2, 14)
    );

    tree.remove(one);

    assert_eq!(
        at(tree.position_by(rule, &NodeErrorOptions::default())),
        (1, 1, 0)
    );
    assert_eq!(
        at(tree.position_by(two, &NodeErrorOptions::default())),
        (3, 2, 14)
    );
}

#[test]
fn range_by_returns_range() {
    let tree = parse("a {  one: X  }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    let (start, end) = tree.range_by(decl, &NodeErrorOptions::default());
    assert_eq!(at(start), (1, 6, 5));
    assert_eq!(at(end), (1, 12, 11));

    let (start, end) = tree.range_by(
        decl,
        &NodeErrorOptions {
            word: Some("one".into()),
            ..Default::default()
        },
    );
    assert_eq!(at(start), (1, 6, 5));
    assert_eq!(at(end), (1, 9, 8));
}

#[test]
fn range_by_returns_range_after_ast_mutations() {
    let mut tree = parse("a {\n\tone: 1;\n\ttwo: 2;}").unwrap();
    let rule = first_rule(&tree);
    let one = tree.first(rule).unwrap();
    let two = tree.next(one).unwrap();

    let check = |tree: &Tree| {
        let (start, end) = tree.range_by(rule, &NodeErrorOptions::default());
        assert_eq!(at(start), (1, 1, 0));
        assert_eq!(at(end), (3, 10, 22));

        let (start, end) = tree.range_by(two, &NodeErrorOptions::default());
        assert_eq!(at(start), (3, 2, 14));
        assert_eq!(at(end), (3, 9, 21));
    };

    check(&tree);
    tree.remove(one);
    check(&tree);
}

#[test]
fn range_by_returns_range_for_index_and_end_index() {
    let tree = parse("a {  one: X  }").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();

    let (start, end) = tree.range_by(
        decl,
        &NodeErrorOptions {
            index: Some(5),
            end_index: Some(7),
            ..Default::default()
        },
    );
    assert_eq!(at(start), (1, 11, 10));
    assert_eq!(at(end), (1, 13, 12));

    // `index: 0` still yields a one-character range.
    let (start, end) = tree.range_by(
        decl,
        &NodeErrorOptions {
            index: Some(0),
            ..Default::default()
        },
    );
    assert_eq!(at(start), (1, 6, 5));
    assert_eq!(at(end), (1, 7, 6));
}

// --- errors --------------------------------------------------------------

#[test]
fn error_generates_custom_error() {
    let tree = parse("a{}").unwrap();
    let rule = first_rule(&tree);
    let error = tree.node_error(
        rule,
        "Test",
        &NodeErrorOptions {
            plugin: Some("plugin".into()),
            ..Default::default()
        },
    );

    assert_eq!(error.message, "plugin: <css input>:1:1: Test");
    assert_eq!(error.reason, "Test");
    assert_eq!(error.plugin.as_deref(), Some("plugin"));
}

#[test]
fn error_generates_custom_error_for_nodes_without_source() {
    let mut tree = Tree::new();
    let rule = tree.create(NewNode::rule("a"));
    let error = tree.node_error(rule, "Test", &NodeErrorOptions::default());
    assert_eq!(error.message, "<css input>: Test");
}

#[test]
fn error_highlights_word() {
    let tree = parse("a{color:black}").unwrap();
    let rule = first_rule(&tree);
    let decl = tree.first(rule).unwrap();
    let error = tree.node_error(
        decl,
        "Test",
        &NodeErrorOptions {
            word: Some("black".into()),
            ..Default::default()
        },
    );

    assert_eq!(error.message, "<css input>:1:9: Test");
    assert_eq!(error.column, Some(9));
    assert_eq!(error.end_column, Some(14));
}
