//! Plugins and the runner that applies them.
//!
//! Ports `lib/processor.js` and the synchronous half of `lib/lazy-result.js`.
//!
//! A plugin implements [`Plugin`]. `once`/`once_exit` see the whole tree; the
//! per-node hooks are driven by a dirty-tracking loop, so a node a plugin
//! changes is visited again — the property the JS visitor API guarantees.

use crate::error::CssSyntaxError;
use crate::map_generator::MapGenerator;
use crate::node::{NodeId, NodeKind};
use crate::options::ProcessOptions;
use crate::result::{PluginContext, Result as ProcessResult};
use crate::tree::Tree;

/// How many times the visitor loop may re-walk a tree before giving up.
const MAX_ROUNDS: usize = 100;

/// A CSS transform.
///
/// Every hook has a no-op default, so a plugin implements only what it needs.
///
/// ```
/// use postcss::{CssSyntaxError, NodeId, Plugin, PluginContext, Processor, Tree};
///
/// struct Prefixer;
///
/// impl Plugin for Prefixer {
///     fn name(&self) -> &str {
///         "prefixer"
///     }
///
///     fn decl(
///         &self,
///         tree: &mut Tree,
///         decl: NodeId,
///         _ctx: &mut PluginContext,
///     ) -> Result<(), CssSyntaxError> {
///         if tree.prop(decl) == Some("user-select") {
///             tree.set_prop(decl, "-webkit-user-select");
///         }
///         Ok(())
///     }
/// }
///
/// let result = Processor::new()
///     .with(Prefixer)
///     .process("a { user-select: none }", Default::default())
///     .unwrap();
/// assert_eq!(result.css, "a { -webkit-user-select: none }");
/// ```
pub trait Plugin {
    /// The plugin's name, used in warnings and error messages.
    fn name(&self) -> &str;

    /// Runs once, before any node visitor.
    fn once(&self, _tree: &mut Tree, _ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
        Ok(())
    }

    /// Runs once, after every node visitor has settled.
    fn once_exit(&self, _tree: &mut Tree, _ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
        Ok(())
    }

    /// Visits the root node.
    fn root(
        &self,
        _tree: &mut Tree,
        _root: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        Ok(())
    }

    /// Visits a rule.
    fn rule(
        &self,
        _tree: &mut Tree,
        _rule: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        Ok(())
    }

    /// Visits an at-rule.
    fn at_rule(
        &self,
        _tree: &mut Tree,
        _at_rule: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        Ok(())
    }

    /// Visits a declaration.
    fn decl(
        &self,
        _tree: &mut Tree,
        _decl: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        Ok(())
    }

    /// Visits a comment.
    fn comment(
        &self,
        _tree: &mut Tree,
        _comment: NodeId,
        _ctx: &mut PluginContext,
    ) -> Result<(), CssSyntaxError> {
        Ok(())
    }
}

/// Runs a list of plugins over CSS.
#[derive(Default)]
pub struct Processor {
    plugins: Vec<Box<dyn Plugin>>,
}

impl Processor {
    /// An empty processor.
    pub fn new() -> Self {
        Processor {
            plugins: Vec::new(),
        }
    }

    /// Adds a plugin, which runs after the ones already added.
    pub fn with(mut self, plugin: impl Plugin + 'static) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Adds a boxed plugin.
    pub fn add(&mut self, plugin: Box<dyn Plugin>) -> &mut Self {
        self.plugins.push(plugin);
        self
    }

    /// Names of the plugins, in run order.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.iter().map(|plugin| plugin.name()).collect()
    }

    /// Parses `css`, runs every plugin, and stringifies the result.
    pub fn process(
        &self,
        css: impl Into<String>,
        opts: ProcessOptions,
    ) -> Result<ProcessResult, CssSyntaxError> {
        let tree = crate::parse_with_options(css, opts.input_options())?;
        self.process_tree(tree, opts)
    }

    /// Runs every plugin over an existing tree.
    pub fn process_tree(
        &self,
        mut tree: Tree,
        opts: ProcessOptions,
    ) -> Result<ProcessResult, CssSyntaxError> {
        let mut ctx = PluginContext::new(opts.clone());

        for plugin in &self.plugins {
            ctx.set_current_plugin(Some(plugin.name().to_string()));
            plugin
                .once(&mut tree, &mut ctx)
                .map_err(|error| with_plugin(error, plugin.name()))?;
        }

        self.run_visitors(&mut tree, &mut ctx)?;

        for plugin in &self.plugins {
            ctx.set_current_plugin(Some(plugin.name().to_string()));
            plugin
                .once_exit(&mut tree, &mut ctx)
                .map_err(|error| with_plugin(error, plugin.name()))?;
        }
        ctx.set_current_plugin(None);

        let (css, map) = MapGenerator::generate(&mut tree, &opts);

        Ok(ProcessResult {
            css,
            map,
            root: tree,
            messages: ctx.messages,
            opts,
        })
    }

    /// Visits every dirty node until the tree settles.
    ///
    /// A node a plugin modifies is marked dirty again and revisited, so plugins
    /// see each other's output. A tree that never settles is an error, as in JS.
    fn run_visitors(&self, tree: &mut Tree, ctx: &mut PluginContext) -> Result<(), CssSyntaxError> {
        if self.plugins.is_empty() {
            return Ok(());
        }

        for round in 0..MAX_ROUNDS {
            let dirty = collect_dirty(tree);
            if dirty.is_empty() {
                return Ok(());
            }

            for id in dirty {
                // The node may have been cleaned or detached by an earlier
                // visitor in this same round.
                if tree.is_clean(id) || is_detached(tree, id) {
                    continue;
                }
                tree.mark_clean(id);

                for plugin in &self.plugins {
                    ctx.set_current_plugin(Some(plugin.name().to_string()));
                    let outcome = match tree.kind(id) {
                        NodeKind::Root | NodeKind::Document => plugin.root(tree, id, ctx),
                        NodeKind::Rule { .. } => plugin.rule(tree, id, ctx),
                        NodeKind::AtRule { .. } => plugin.at_rule(tree, id, ctx),
                        NodeKind::Decl { .. } => plugin.decl(tree, id, ctx),
                        NodeKind::Comment { .. } => plugin.comment(tree, id, ctx),
                    };
                    outcome.map_err(|error| with_plugin(error, plugin.name()))?;

                    if is_detached(tree, id) {
                        break;
                    }
                }
            }

            if round == MAX_ROUNDS - 1 {
                return Err(CssSyntaxError::new(
                    "Unstable CSS AST: plugins kept changing nodes after 100 visitor rounds",
                ));
            }
        }

        Ok(())
    }
}

/// Nodes needing a visit, in document order, root first.
fn collect_dirty(tree: &Tree) -> Vec<NodeId> {
    let root = tree.root();
    let mut dirty = Vec::new();
    if !tree.is_clean(root) {
        dirty.push(root);
    }
    tree.walk_ref(root, |tree, node| {
        if !tree.is_clean(node) {
            dirty.push(node);
        }
    });
    dirty
}

fn is_detached(tree: &Tree, id: NodeId) -> bool {
    id != tree.root() && tree.parent(id).is_none()
}

fn with_plugin(mut error: CssSyntaxError, plugin: &str) -> CssSyntaxError {
    error.set_plugin(plugin);
    error
}
