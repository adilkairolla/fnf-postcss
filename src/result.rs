//! The output of a processing run: CSS, source map, and the messages plugins
//! left behind.
//!
//! Ports `lib/result.js` and `lib/warning.js`.

use crate::node::NodeId;
use crate::options::ProcessOptions;
use crate::source_map::SourceMapGenerator;
use crate::tree::{NodeErrorOptions, Tree};

/// A warning a plugin attached to the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// Human-readable warning text.
    pub text: String,
    /// Plugin that emitted the warning.
    pub plugin: Option<String>,
    /// Node the warning is about.
    pub node: Option<NodeId>,
    /// 1-based start line.
    pub line: Option<usize>,
    /// 1-based start column.
    pub column: Option<usize>,
    /// 1-based end line.
    pub end_line: Option<usize>,
    /// 1-based end column.
    pub end_column: Option<usize>,
    /// The word the warning points at, if one was given.
    pub word: Option<String>,
    /// The index the warning points at, if one was given.
    pub index: Option<usize>,
}

impl Warning {
    /// Builds a warning about a node, resolving its position.
    pub fn new(
        tree: &Tree,
        text: impl Into<String>,
        node: Option<NodeId>,
        opts: &NodeErrorOptions,
    ) -> Self {
        let mut warning = Warning {
            text: text.into(),
            plugin: opts.plugin.clone(),
            node,
            line: None,
            column: None,
            end_line: None,
            end_column: None,
            word: opts.word.clone(),
            index: opts.index,
        };

        if let Some(node) = node {
            if tree.source(node).is_some() {
                let (start, end) = tree.range_by(node, opts);
                warning.line = Some(start.line);
                warning.column = Some(start.column);
                warning.end_line = Some(end.line);
                warning.end_column = Some(end.column);
            }
        }

        warning
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.plugin, self.line, self.column) {
            (Some(plugin), Some(line), Some(column)) => {
                write!(f, "{}:{}:{}: {}", plugin, line, column, self.text)
            }
            (Some(plugin), _, _) => write!(f, "{}: {}", plugin, self.text),
            (None, Some(line), Some(column)) => {
                write!(f, "{}:{}: {}", line, column, self.text)
            }
            _ => f.write_str(&self.text),
        }
    }
}

/// Anything a plugin can leave on the result.
///
/// PostCSS lets plugins push arbitrary objects onto `result.messages`; the
/// common cases are warnings and dependency declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// A warning for the user.
    Warning(Warning),
    /// A file the output depends on, as `postcss-import` and friends report.
    Dependency {
        /// Plugin that reported it.
        plugin: Option<String>,
        /// Absolute path of the file depended on.
        file: String,
        /// The file that pulled it in.
        parent: Option<String>,
    },
    /// A directory glob the output depends on.
    DirDependency {
        /// Plugin that reported it.
        plugin: Option<String>,
        /// Absolute path of the directory depended on.
        dir: String,
        /// Glob restricting which files matter.
        glob: Option<String>,
        /// The file that pulled it in.
        parent: Option<String>,
    },
    /// Any other message.
    Custom {
        /// The message's `type`.
        kind: String,
        /// Plugin that reported it.
        plugin: Option<String>,
        /// Anything else the plugin attached.
        data: serde_json::Value,
    },
}

impl Message {
    /// The `type` field PostCSS uses.
    pub fn kind(&self) -> &str {
        match self {
            Message::Warning(_) => "warning",
            Message::Dependency { .. } => "dependency",
            Message::DirDependency { .. } => "dir-dependency",
            Message::Custom { kind, .. } => kind,
        }
    }
}

/// Everything plugins can read and write during a run, besides the tree itself.
///
/// Split out from [`Result`] so a plugin can hold the tree mutably and still
/// record warnings.
#[derive(Clone, Debug, Default)]
pub struct PluginContext {
    /// Messages collected so far.
    pub messages: Vec<Message>,
    /// The run's options.
    pub opts: ProcessOptions,
    current_plugin: Option<String>,
}

impl PluginContext {
    /// An empty context for a run with these options.
    pub fn new(opts: ProcessOptions) -> Self {
        PluginContext {
            messages: Vec::new(),
            opts,
            current_plugin: None,
        }
    }

    pub(crate) fn set_current_plugin(&mut self, plugin: Option<String>) {
        self.current_plugin = plugin;
    }

    /// Records a warning about a node.
    ///
    /// Port of `Result#warn()`: the plugin name is filled in automatically.
    pub fn warn(
        &mut self,
        tree: &Tree,
        text: impl Into<String>,
        node: Option<NodeId>,
        opts: &NodeErrorOptions,
    ) -> &Warning {
        let mut opts = opts.clone();
        if opts.plugin.is_none() {
            opts.plugin = self.current_plugin.clone();
        }
        let warning = Warning::new(tree, text, node, &opts);
        self.messages.push(Message::Warning(warning));
        match self.messages.last() {
            Some(Message::Warning(warning)) => warning,
            _ => unreachable!("just pushed a warning"),
        }
    }

    /// Records a warning with no position.
    pub fn warn_text(&mut self, text: impl Into<String>) {
        let plugin = self.current_plugin.clone();
        self.messages.push(Message::Warning(Warning {
            text: text.into(),
            plugin,
            node: None,
            line: None,
            column: None,
            end_line: None,
            end_column: None,
            word: None,
            index: None,
        }));
    }

    /// Records a non-warning message.
    pub fn message(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Only the warnings, in order.
    pub fn warnings(&self) -> Vec<&Warning> {
        self.messages
            .iter()
            .filter_map(|message| match message {
                Message::Warning(warning) => Some(warning),
                _ => None,
            })
            .collect()
    }
}

/// The result of [`crate::Processor::process`].
#[derive(Debug)]
pub struct Result {
    /// The transformed CSS.
    pub css: String,
    /// The generated source map, when one was requested and not inlined.
    pub map: Option<SourceMapGenerator>,
    /// The transformed tree.
    pub root: Tree,
    /// Messages plugins recorded.
    pub messages: Vec<Message>,
    /// The options the run used.
    pub opts: ProcessOptions,
}

impl Result {
    /// Alias of [`Result::css`], as in the JS API.
    pub fn content(&self) -> &str {
        &self.css
    }

    /// Only the warnings, in order.
    pub fn warnings(&self) -> Vec<&Warning> {
        self.messages
            .iter()
            .filter_map(|message| match message {
                Message::Warning(warning) => Some(warning),
                _ => None,
            })
            .collect()
    }

    /// The source map as JSON text.
    pub fn map_json(&self) -> Option<String> {
        self.map.as_ref().map(|map| map.to_json_string())
    }
}

impl std::fmt::Display for Result {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.css)
    }
}
