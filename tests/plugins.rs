//! The plugin API: hook order, the revisit loop, warnings and errors.
//!
//! Corresponds to the parts of the upstream `processor.test.ts`,
//! `visitor.test.ts`, `result.test.ts` and `warning.test.ts` that survive the
//! move to a synchronous, trait-based plugin API.

// `CssSyntaxError` is deliberately large; see the note in `lib.rs`.
#![allow(clippy::result_large_err)]

use postcss::{
    CssSyntaxError, Message, NewNode, NodeErrorOptions, NodeId, Plugin, PluginContext,
    ProcessOptions, Processor, Tree,
};

/// A plugin that appends its hook calls to a shared log.
struct Tracer {
    name: &'static str,
    log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Plugin for Tracer {
    fn name(&self) -> &str {
        self.name
    }

    fn once(&self, _tree: &mut Tree, _ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
        self.log.lock().unwrap().push(format!("{}:once", self.name));
        Ok(())
    }

    fn once_exit(&self, _tree: &mut Tree, _ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:onceExit", self.name));
        Ok(())
    }

    fn rule(
        &self,
        _tree: &mut Tree,
        _rule: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        self.log.lock().unwrap().push(format!("{}:rule", self.name));
        Ok(())
    }

    fn decl(
        &self,
        _tree: &mut Tree,
        _decl: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        self.log.lock().unwrap().push(format!("{}:decl", self.name));
        Ok(())
    }
}

#[test]
fn runs_hooks_in_order() {
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    Processor::new()
        .with(Tracer {
            name: "one",
            log: log.clone(),
        })
        .with(Tracer {
            name: "two",
            log: log.clone(),
        })
        .process("a { color: red }", ProcessOptions::default())
        .unwrap();

    let log = log.lock().unwrap().clone();
    assert_eq!(
        log,
        [
            // Every `once` runs first, in plugin order.
            "one:once",
            "two:once",
            // Then each node is visited by every plugin, root first.
            "one:rule",
            "two:rule",
            "one:decl",
            "two:decl",
            // `onceExit` runs last.
            "one:onceExit",
            "two:onceExit",
        ]
    );
}

/// Rewrites `color` into `background-color`.
struct Renamer;

impl Plugin for Renamer {
    fn name(&self) -> &str {
        "renamer"
    }

    fn decl(
        &self,
        tree: &mut Tree,
        decl: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        if tree.prop(decl) == Some("color") {
            tree.set_prop(decl, "background-color");
        }
        Ok(())
    }
}

#[test]
fn applies_a_declaration_visitor() {
    let result = Processor::new()
        .with(Renamer)
        .process("a { color: red }", ProcessOptions::default())
        .unwrap();

    assert_eq!(result.css, "a { background-color: red }");
}

/// Adds a vendor-prefixed copy of every `user-select`, once.
struct Prefixer;

impl Plugin for Prefixer {
    fn name(&self) -> &str {
        "prefixer"
    }

    fn decl(
        &self,
        tree: &mut Tree,
        decl: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        if tree.prop(decl) != Some("user-select") {
            return Ok(());
        }
        let prefixed = format!("-webkit-{}", tree.prop(decl).unwrap());
        let already = tree
            .prev(decl)
            .and_then(|prev| tree.prop(prev).map(|prop| prop == prefixed))
            .unwrap_or(false);
        if !already {
            let value = tree.value(decl).unwrap_or_default().to_string();
            tree.insert_before(decl, NewNode::decl(prefixed, value))?;
        }
        Ok(())
    }
}

#[test]
fn visits_nodes_a_plugin_inserted() {
    // The inserted declaration must itself be visited, or the second plugin
    // would never see it.
    struct Uppercase;
    impl Plugin for Uppercase {
        fn name(&self) -> &str {
            "uppercase"
        }
        fn decl(
            &self,
            tree: &mut Tree,
            decl: NodeId,
            _ctx: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            let value = tree.value(decl).unwrap_or_default().to_uppercase();
            if tree.value(decl) != Some(value.as_str()) {
                tree.set_value(decl, value);
            }
            Ok(())
        }
    }

    let result = Processor::new()
        .with(Prefixer)
        .with(Uppercase)
        .process("a { user-select: none }", ProcessOptions::default())
        .unwrap();

    assert_eq!(
        result.css,
        "a { -webkit-user-select: NONE; user-select: NONE }"
    );
}

#[test]
fn visits_nodes_added_in_once() {
    struct Adder;
    impl Plugin for Adder {
        fn name(&self) -> &str {
            "adder"
        }
        fn once(&self, tree: &mut Tree, _ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
            let root = tree.root();
            tree.append(root, NewNode::rule("b").child(NewNode::decl("top", "0")))?;
            Ok(())
        }
    }

    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    Processor::new()
        .with(Adder)
        .with(Tracer {
            name: "tracer",
            log: log.clone(),
        })
        .process("a {}", ProcessOptions::default())
        .unwrap();

    let log = log.lock().unwrap().clone();
    assert_eq!(
        log,
        [
            "tracer:once",
            "tracer:rule",
            "tracer:rule",
            "tracer:decl",
            "tracer:onceExit"
        ]
    );
}

#[test]
fn removing_a_node_stops_further_visits_of_it() {
    struct Remover;
    impl Plugin for Remover {
        fn name(&self) -> &str {
            "remover"
        }
        fn decl(
            &self,
            tree: &mut Tree,
            decl: NodeId,
            _ctx: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            tree.remove(decl);
            Ok(())
        }
    }

    struct Counter {
        seen: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Plugin for Counter {
        fn name(&self) -> &str {
            "counter"
        }
        fn decl(
            &self,
            _tree: &mut Tree,
            _decl: NodeId,
            _ctx: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            self.seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let result = Processor::new()
        .with(Remover)
        .with(Counter { seen: seen.clone() })
        .process("a { color: red; top: 0 }", ProcessOptions::default())
        .unwrap();

    assert_eq!(result.css, "a { }");
    assert_eq!(
        seen.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "a detached node is not handed to later plugins"
    );
}

#[test]
fn collects_warnings_with_positions() {
    struct Warner;
    impl Plugin for Warner {
        fn name(&self) -> &str {
            "warner"
        }
        fn decl(
            &self,
            tree: &mut Tree,
            decl: NodeId,
            ctx: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            ctx.warn(
                tree,
                "Avoid !important",
                Some(decl),
                &NodeErrorOptions {
                    word: Some("red".into()),
                    ..Default::default()
                },
            );
            Ok(())
        }
    }

    let result = Processor::new()
        .with(Warner)
        .process("a {\n  color: red;\n}", ProcessOptions::default())
        .unwrap();

    let warnings = result.warnings();
    assert_eq!(warnings.len(), 1);
    let warning = warnings[0];
    assert_eq!(warning.text, "Avoid !important");
    // The plugin name is filled in automatically.
    assert_eq!(warning.plugin.as_deref(), Some("warner"));
    assert_eq!((warning.line, warning.column), (Some(2), Some(10)));
    assert_eq!((warning.end_line, warning.end_column), (Some(2), Some(13)));
    assert_eq!(warning.to_string(), "warner:2:10: Avoid !important");
}

#[test]
fn collects_custom_messages() {
    struct Dependencies;
    impl Plugin for Dependencies {
        fn name(&self) -> &str {
            "deps"
        }
        fn once(&self, _tree: &mut Tree, ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
            ctx.message(Message::Dependency {
                plugin: Some("deps".into()),
                file: "/a/b.css".into(),
                parent: None,
            });
            Ok(())
        }
    }

    let result = Processor::new()
        .with(Dependencies)
        .process("a{}", ProcessOptions::default())
        .unwrap();

    assert_eq!(result.messages.len(), 1);
    assert_eq!(result.messages[0].kind(), "dependency");
    assert!(result.warnings().is_empty());
}

#[test]
fn attaches_the_plugin_name_to_errors() {
    struct Failing;
    impl Plugin for Failing {
        fn name(&self) -> &str {
            "failing"
        }
        fn decl(
            &self,
            tree: &mut Tree,
            decl: NodeId,
            _ctx: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            Err(tree.node_error(decl, "Bad declaration", &NodeErrorOptions::default()))
        }
    }

    let error = Processor::new()
        .with(Failing)
        .process("a { color: red }", ProcessOptions::default())
        .unwrap_err();

    assert_eq!(error.reason, "Bad declaration");
    assert_eq!(error.plugin.as_deref(), Some("failing"));
    assert_eq!(error.message, "failing: <css input>:1:5: Bad declaration");
}

#[test]
fn reports_a_syntax_error_before_running_plugins() {
    let error = Processor::new()
        .with(Renamer)
        .process("a {", ProcessOptions::default())
        .unwrap_err();
    assert_eq!(error.reason, "Unclosed block");
}

#[test]
fn detects_a_tree_that_never_settles() {
    // A plugin that always dirties its node again can never converge.
    struct Flipper;
    impl Plugin for Flipper {
        fn name(&self) -> &str {
            "flipper"
        }
        fn decl(
            &self,
            tree: &mut Tree,
            decl: NodeId,
            _ctx: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            let value = tree.value(decl).unwrap_or_default().to_string();
            tree.set_value(decl, format!("{}x", value));
            Ok(())
        }
    }

    let error = Processor::new()
        .with(Flipper)
        .process("a { color: red }", ProcessOptions::default())
        .unwrap_err();
    assert!(
        error.reason.contains("Unstable CSS AST"),
        "unexpected reason: {}",
        error.reason
    );
}

#[test]
fn exposes_plugin_names_and_result_fields() {
    let processor = Processor::new().with(Renamer).with(Prefixer);
    assert_eq!(processor.plugin_names(), ["renamer", "prefixer"]);

    let result = processor
        .process(
            "a { color: red }",
            ProcessOptions::default().from("a.css").to("b.css"),
        )
        .unwrap();

    assert_eq!(result.content(), result.css);
    assert_eq!(result.opts.to.as_deref(), Some("b.css"));
    // The transformed tree is available for further work.
    assert_eq!(result.root.children(result.root.root()).len(), 1);
}

#[test]
fn processes_an_existing_tree() {
    let tree = postcss::parse("a { color: red }").unwrap();
    let result = Processor::new()
        .with(Renamer)
        .process_tree(tree, ProcessOptions::default())
        .unwrap();
    assert_eq!(result.css, "a { background-color: red }");
}

#[test]
fn runs_with_no_plugins() {
    let result = Processor::new()
        .process("a { color: red }", ProcessOptions::default())
        .unwrap();
    assert_eq!(result.css, "a { color: red }");
    assert!(result.messages.is_empty());
}

#[test]
fn visits_every_node_type() {
    /// A plugin takes `&self`, so shared state is how it reports back.
    struct All {
        seen: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    impl All {
        fn note(&self, kind: &'static str) -> Result<(), CssSyntaxError> {
            self.seen.lock().unwrap().push(kind);
            Ok(())
        }
    }

    impl Plugin for All {
        fn name(&self) -> &str {
            "all"
        }
        fn root(
            &self,
            _t: &mut Tree,
            _n: NodeId,
            _c: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            self.note("root")
        }
        fn rule(
            &self,
            _t: &mut Tree,
            _n: NodeId,
            _c: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            self.note("rule")
        }
        fn at_rule(
            &self,
            _t: &mut Tree,
            _n: NodeId,
            _c: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            self.note("atrule")
        }
        fn decl(
            &self,
            _t: &mut Tree,
            _n: NodeId,
            _c: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            self.note("decl")
        }
        fn comment(
            &self,
            _t: &mut Tree,
            _n: NodeId,
            _c: &mut PluginContext,
        ) -> Result<(), CssSyntaxError> {
            self.note("comment")
        }
    }

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    Processor::new()
        .with(All { seen: seen.clone() })
        .process(
            "/* c */ @media screen { a { top: 0 } }",
            ProcessOptions::default(),
        )
        .unwrap();

    assert_eq!(
        seen.lock().unwrap().clone(),
        ["root", "comment", "atrule", "rule", "decl"]
    );
}
