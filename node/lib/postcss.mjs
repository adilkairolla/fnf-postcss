// Part of the PostCSS-compatible JS layer of `fnf-postcss`, taken from PostCSS
// (https://github.com/postcss/postcss, MIT, Copyright 2013 Andrey Sitnik) so
// that plugins see exactly the object model they were written against. Parsing,
// stringification, source maps and tokenizing are handled by the Rust core; see
// lib/parse.js, lib/map-generator.js and lib/source-map.js.

import postcss from './postcss.js'

export default postcss

export const stringify = postcss.stringify
export const fromJSON = postcss.fromJSON
export const plugin = postcss.plugin
export const parse = postcss.parse
export const list = postcss.list

export const document = postcss.document
export const comment = postcss.comment
export const atRule = postcss.atRule
export const rule = postcss.rule
export const decl = postcss.decl
export const root = postcss.root

export const CssSyntaxError = postcss.CssSyntaxError
export const Declaration = postcss.Declaration
export const Container = postcss.Container
export const Processor = postcss.Processor
export const Document = postcss.Document
export const Comment = postcss.Comment
export const Warning = postcss.Warning
export const AtRule = postcss.AtRule
export const Result = postcss.Result
export const Input = postcss.Input
export const Rule = postcss.Rule
export const Root = postcss.Root
export const Node = postcss.Node
