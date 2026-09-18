//! Code-block tokeniser. Language data lives in `dialects.rs`; this file is the shared scanner.
//!
//! Roles emitted (the complete set; clients map each to a colour):
//! keyword, type, constant, function, string, number, comment, operator, punctuation,
//! preprocessor, attribute, variable, tag, key, heading, inserted, deleted.
//! `attribute` covers annotations, markup attribute names and markup emphasis;
//! `operator` and `punctuation` are emitted for every dialect whose body is code, not prose.

use crate::dialects::{
    self, Dialect, ATTR, CHAR, DECOR, DIFF, EMBED, ICASE, KEYS, MARKUP, MD, NEST, PREFIX, PREPROC,
    REGEX, RST, TABLE, TEX, TEXTILE, TRIPLE,
};
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub role: &'static str,
}

pub(crate) fn highlight(source: &str, language: &str, base: u32) -> Vec<Token> {
    dialects::find(language).map_or_else(Vec::new, |d| tokenize(source, d, base))
}

/// Fence tags that mean "show this verbatim": no chip, no highlighting.
pub fn plain(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "text" | "txt" | "plain" | "plaintext" | "none"
    )
}

/// Presentation-time guess for a fence that carried no language; never touches the canonical body.
pub fn detect(source: &str) -> Option<&'static str> {
    let head = &source[..boundary(source, source.len().min(4096))];
    let trimmed = head.trim_start();
    if trimmed.len() < 8 {
        return None;
    }
    signal(trimmed, head).or_else(|| score(head))
}

struct Out<'a> {
    src: &'a str,
    at: u32,
    done: usize,
    tokens: Vec<Token>,
}
impl Out<'_> {
    fn mark(&mut self, to: usize) -> u32 {
        self.at += self.src[self.done..to].encode_utf16().count() as u32;
        self.done = to;
        self.at
    }
    fn push(&mut self, from: usize, to: usize, role: &'static str) {
        let from = boundary(self.src, from).max(self.done);
        let to = boundary(self.src, to);
        if to <= from {
            return;
        }
        let start = self.mark(from);
        let end = self.mark(to);
        self.tokens.push(Token { start, end, role });
    }
}

fn boundary(src: &str, to: usize) -> usize {
    let mut to = to.min(src.len());
    while !src.is_char_boundary(to) {
        to += 1;
    }
    to
}
fn line_head(b: &[u8], i: usize) -> bool {
    b[..i]
        .iter()
        .rev()
        .take_while(|&&c| c != b'\n')
        .all(|&c| c == b' ' || c == b'\t')
}
/// A key may sit behind indentation and one sequence bullet (`- name:`).
fn key_head(b: &[u8], i: usize) -> bool {
    let start = b[..i].iter().rposition(|&c| c == b'\n').map_or(0, |p| p + 1);
    let mut s = &b[start..i];
    while matches!(s.first(), Some(b' ' | b'\t')) {
        s = &s[1..];
    }
    if s.first() == Some(&b'-') {
        s = &s[1..];
        while s.first() == Some(&b' ') {
            s = &s[1..];
        }
    }
    s.is_empty()
}
fn line_end(b: &[u8], i: usize) -> usize {
    b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| i + p)
}
/// `extra` holds the dialect's further identifier characters, so `if-let` and `set!` stay one word.
fn word_end(src: &str, i: usize, extra: &str) -> usize {
    let mut j = i;
    for ch in src[i..].chars() {
        if ch.is_alphanumeric() || ch == '_' || extra.contains(ch) {
            j += ch.len_utf8();
        } else {
            break;
        }
    }
    j
}
fn after_space(b: &[u8], mut i: usize) -> usize {
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    i
}
/// Byte index just past `close`, searched on the current line only.
fn paired(src: &str, i: usize, open: &str, close: &str) -> Option<usize> {
    let rest = &src[i + open.len()..];
    let stop = rest.find('\n').unwrap_or(rest.len());
    let at = rest[..stop].find(close).filter(|p| *p > 0)?;
    Some(i + open.len() + at + close.len())
}
fn starts(src: &str, i: usize, token: &str, icase: bool) -> bool {
    let rest = &src[i..];
    if icase {
        rest.len() >= token.len() && rest.as_bytes()[..token.len()].eq_ignore_ascii_case(token.as_bytes())
    } else {
        rest.starts_with(token)
    }
}
fn char_literal(source: &str) -> bool {
    let rest = &source[1..];
    rest.starts_with('\\')
        || rest
            .chars()
            .next()
            .is_some_and(|ch| rest.as_bytes().get(ch.len_utf8()) == Some(&b'\''))
}

fn tokenize(source: &str, d: &Dialect, base: u32) -> Vec<Token> {
    let bytes = source.as_bytes();
    let icase = d.has(ICASE);
    let trimmed = source.trim_start();
    // A prose body (markdown, diff, markup content, LaTeX) has no operators to mark and no calls.
    let marks = !(d.has(MD | RST | TEXTILE | DIFF | TEX) || d.has(MARKUP) && d.keywords.is_empty());
    let calls = !d.keywords.is_empty();
    let hunks = d.has(DIFF)
        && (source.contains("@@")
            || ["---", "+++", "diff ", "Index:"]
                .iter()
                .any(|p| trimmed.starts_with(p)));
    let mut out = Out {
        src: source,
        at: base,
        done: 0,
        tokens: Vec::new(),
    };
    let mut i = 0;
    while i < bytes.len() {
        let head = line_head(bytes, i);
        // Whole-line constructs.
        if head {
            let stop = line_end(bytes, i);
            if hunks {
                let role = if ["---", "+++", "@@", "diff ", "index ", "Index:", "==="]
                    .iter()
                    .any(|p| source[i..].starts_with(p))
                {
                    Some("heading")
                } else if bytes[i] == b'+' {
                    Some("inserted")
                } else if bytes[i] == b'-' {
                    Some("deleted")
                } else {
                    None
                };
                if let Some(role) = role {
                    out.push(i, stop, role);
                    i = stop;
                    continue;
                }
            }
            if d.has(TABLE) && bytes[i] == b'[' {
                out.push(i, stop, "heading");
                i = stop;
                continue;
            }
            if d.has(PREPROC) && bytes[i] == b'#' {
                out.push(i, stop, "preprocessor");
                i = stop;
                continue;
            }
            if d.has(MD) {
                if bytes[i] == b'#' {
                    out.push(i, stop, "heading");
                    i = stop;
                    continue;
                }
                if bytes[i] == b'>' {
                    out.push(i, stop, "comment");
                    i = stop;
                    continue;
                }
                if source[i..].starts_with("```") || source[i..].starts_with("~~~") {
                    out.push(i, stop, "punctuation");
                    i = stop;
                    continue;
                }
            }
            if (d.has(MD) || d.has(RST)) && rule_line(&source[i..stop]) {
                out.push(i, stop, "heading");
                i = stop;
                continue;
            }
            if d.has(RST) && source[i..].starts_with("..") {
                out.push(i, stop, if source[i..stop].contains("::") { "preprocessor" } else { "comment" });
                i = stop;
                continue;
            }
            if d.has(RST) && bytes[i] == b':' {
                if let Some(end) = paired(source, i, ":", ":") {
                    out.push(i, end, "key");
                    i = end;
                    continue;
                }
            }
            if d.has(TEXTILE) {
                let marker = &source[i..boundary(source, i + 3).min(stop)];
                if marker.len() == 3
                    && marker.as_bytes()[0] == b'h'
                    && marker.as_bytes()[1].is_ascii_digit()
                    && marker.as_bytes()[2] == b'.'
                {
                    out.push(i, stop, "heading");
                    i = stop;
                    continue;
                }
            }
        }
        // Comments.
        if let Some((open, close)) = d.block.iter().find(|(open, _)| bytes[i..].starts_with(open.as_bytes())) {
            let mut j = i + open.len();
            let mut depth = 1usize;
            while j < bytes.len() && depth > 0 {
                if bytes[j..].starts_with(close.as_bytes()) {
                    depth -= 1;
                    j += close.len();
                } else if d.has(NEST) && bytes[j..].starts_with(open.as_bytes()) {
                    depth += 1;
                    j += open.len();
                } else {
                    j += 1;
                }
            }
            out.push(i, j, "comment");
            i = boundary(source, j);
            continue;
        }
        if d.line.iter().any(|t| {
            let alpha = t.as_bytes()[0].is_ascii_alphabetic();
            starts(source, i, t, icase)
                && (!alpha
                    || head
                        && !matches!(bytes.get(i + t.len()), Some(c) if c.is_ascii_alphanumeric() || *c == b'_'))
        }) {
            let stop = line_end(bytes, i);
            out.push(i, stop, "comment");
            i = stop;
            continue;
        }
        // Embedded-template delimiters, then markup tags.
        if d.has(EMBED) {
            if let Some(token) = ["<?php", "<?=", "<?", "<%=", "<%-", "<%", "-%>", "%>", "?>"]
                .iter()
                .find(|t| starts(source, i, t, true))
            {
                out.push(i, i + token.len(), "tag");
                i += token.len();
                continue;
            }
        }
        if d.has(MARKUP) && bytes[i] == b'<' && tag_start(bytes, i) {
            i = boundary(source, tag(&mut out, source, i));
            continue;
        }
        if d.has(REGEX) {
            i = boundary(source, regex_atom(&mut out, source, i));
            continue;
        }
        if d.has(TEX) && bytes[i] == b'\\' {
            let mut j = word_end(source, i + 1, d.word);
            if j == i + 1 {
                j = boundary(source, i + 2);
            }
            out.push(i, j, "keyword");
            if matches!(&source[i..j], "\\begin" | "\\end") && bytes.get(j) == Some(&b'{') {
                if let Some(end) = paired(source, j, "{", "}") {
                    out.push(j, end, "type");
                    j = end;
                }
            }
            i = j;
            continue;
        }
        if d.has(ATTR) && bytes[i] == b'#' && matches!(bytes.get(i + 1), Some(b'[') | Some(b'!')) {
            let open = if bytes[i + 1] == b'!' { i + 2 } else { i + 1 };
            if bytes.get(open) == Some(&b'[') {
                let mut j = open + 1;
                let mut depth = 1;
                while j < bytes.len() && depth > 0 {
                    match bytes[j] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        _ => (),
                    }
                    j += 1;
                }
                out.push(i, j, "attribute");
                i = j;
                continue;
            }
        }
        if d.has(DECOR) && bytes[i] == b'@' {
            let mut j = word_end(source, i + 1, d.word);
            while matches!(bytes.get(j), Some(b'-')) && word_end(source, j + 1, d.word) > j + 1 {
                j = word_end(source, j + 1, d.word);
            }
            if j > i + 1 {
                let role = if d.known(&source[i + 1..j]) { "keyword" } else { "attribute" };
                out.push(i, j, role);
                i = j;
                continue;
            }
        }
        if !d.sigils.is_empty() && d.sigils.as_bytes().contains(&bytes[i]) {
            let sigil = bytes[i];
            let mut j = i + 1;
            if sigil == b'@' && bytes.get(j) == Some(&b'@') {
                j += 1;
            }
            match bytes.get(j) {
                Some(b'{') => j = paired(source, j, "{", "}").unwrap_or(j),
                Some(b'(') => j = paired(source, j, "(", ")").unwrap_or(j),
                _ => j = word_end(source, j, d.word),
            }
            if j > i + 1 {
                if sigil == b'%' && bytes.get(j) == Some(&b'%') {
                    j += 1;
                }
                out.push(i, j, "variable");
                i = j;
                continue;
            }
        }
        // Strings, with data-format keys taking the key role.
        if let Some(end) = string_at(source, i, d) {
            let next = after_space(bytes, end);
            let role = if d.has(KEYS) && bytes.get(next) == Some(&b':') {
                "key"
            } else {
                "string"
            };
            out.push(i, end, role);
            i = boundary(source, end);
            continue;
        }
        if bytes[i].is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric()
                    || bytes[j] == b'_'
                    || bytes[j] == b'.' && bytes.get(j + 1).is_some_and(u8::is_ascii_digit)
                    || matches!(bytes[j], b'+' | b'-')
                        && matches!(bytes[j - 1], b'e' | b'E' | b'p' | b'P'))
            {
                j += 1;
            }
            out.push(i, j, "number");
            i = j;
            continue;
        }
        if d.has(KEYS) && key_head(bytes, i) {
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || b"_-./%+*".contains(&bytes[j]))
            {
                j += 1;
            }
            let sep = after_space(bytes, j);
            if j > i
                && (bytes.get(sep) == Some(&b'=')
                    || bytes.get(sep) == Some(&b':') && bytes.get(sep + 1) != Some(&b':'))
            {
                out.push(i, j, "key");
                i = j;
                continue;
            }
        }
        if d.has(MD) || d.has(RST) || d.has(TEXTILE) {
            if let Some((end, role)) = inline_markup(source, i, d) {
                out.push(i, end, role);
                i = end;
                continue;
            }
        }
        let ch = source[i..].chars().next().unwrap();
        if ch.is_alphabetic() || ch == '_' {
            let j = word_end(source, i, d.word);
            let word = &source[i..j];
            let role = d.role(word).or_else(|| {
                let call = calls
                    && (bytes.get(j) == Some(&b'(')
                        || bytes.get(j) == Some(&b'!') && bytes.get(j + 1) == Some(&b'('));
                let prefixed = d.has(PREFIX)
                    && bytes[..i]
                        .iter()
                        .rposition(|&c| !c.is_ascii_whitespace())
                        .is_some_and(|p| bytes[p] == b'(');
                (call || prefixed).then_some("function")
            });
            if let Some(role) = role {
                out.push(i, j, role);
            }
            i = j;
            continue;
        }
        if marks && BRACKETS.contains(&bytes[i]) {
            out.push(i, i + 1, "punctuation");
            i += 1;
            continue;
        }
        if marks && OPERATORS.contains(&bytes[i]) {
            let mut j = i;
            while j < bytes.len() && OPERATORS.contains(&bytes[j]) {
                j += 1;
            }
            out.push(i, j, "operator");
            i = j;
            continue;
        }
        i += ch.len_utf8();
    }
    out.tokens
}

const BRACKETS: &[u8] = b"()[]{},;";
const OPERATORS: &[u8] = b"+-*/%=<>!&|^~?:.";

fn rule_line(line: &str) -> bool {
    let line = line.trim_end();
    line.len() >= 2
        && line
            .bytes()
            .all(|c| c == line.as_bytes()[0] && b"=-~^\"'*+#`:.".contains(&c))
}

fn tag_start(bytes: &[u8], i: usize) -> bool {
    matches!(bytes.get(i + 1), Some(b'/' | b'!' | b'?'))
        || bytes.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic())
}

fn tag(out: &mut Out, source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    let mut j = i + 1;
    if matches!(bytes.get(j), Some(b'/' | b'!' | b'?')) {
        j += 1;
    }
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'-' | b'_' | b':' | b'.')) {
        j += 1;
    }
    out.push(i, j, "tag");
    while j < bytes.len() {
        match bytes[j] {
            b' ' | b'\t' | b'\r' | b'\n' | b'=' => j += 1,
            b'>' => {
                out.push(j, j + 1, "tag");
                return j + 1;
            }
            b'/' if bytes.get(j + 1) == Some(&b'>') => {
                out.push(j, j + 2, "tag");
                return j + 2;
            }
            b'"' | b'\'' => {
                let quote = bytes[j];
                let mut k = j + 1;
                while k < bytes.len() && bytes[k] != quote {
                    k += 1;
                }
                let k = (k + 1).min(bytes.len());
                out.push(j, k, "string");
                j = k;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let mut k = j;
                while k < bytes.len()
                    && (bytes[k].is_ascii_alphanumeric() || matches!(bytes[k], b'-' | b'_' | b':' | b'.'))
                {
                    k += 1;
                }
                out.push(j, k, "attribute");
                j = k;
            }
            _ => j += 1,
        }
    }
    j
}

fn regex_atom(out: &mut Out, source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    match bytes[i] {
        b'\\' => {
            let end = boundary(source, i + 2);
            out.push(i, end, "constant");
            end
        }
        b'[' => {
            let mut j = i + 1;
            if bytes.get(j) == Some(&b'^') {
                j += 1;
            }
            while j < bytes.len() && bytes[j] != b']' {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            let j = (j + 1).min(bytes.len());
            out.push(i, j, "type");
            j
        }
        b'{' => {
            let end = paired(source, i, "{", "}").unwrap_or(i + 1);
            out.push(i, end, "operator");
            end
        }
        b'(' | b')' => {
            let mut j = i + 1;
            while matches!(bytes.get(j), Some(b'?') | Some(b':') | Some(b'<') | Some(b'=') | Some(b'!')) {
                j += 1;
            }
            out.push(i, j, "punctuation");
            j
        }
        b'*' | b'+' | b'?' | b'|' | b'^' | b'$' | b'.' => {
            out.push(i, i + 1, "operator");
            i + 1
        }
        _ => boundary(source, i + 1),
    }
}

/// Emphasis, literals and links in the lightweight markup dialects.
fn inline_markup(source: &str, i: usize, d: &Dialect) -> Option<(usize, &'static str)> {
    let bytes = source.as_bytes();
    if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'\\') {
        return None;
    }
    let pairs: &[(&str, &str, &str)] = if d.has(TEXTILE) {
        &[("@", "@", "string"), ("*", "*", "attribute"), ("_", "_", "attribute")]
    } else if d.has(RST) {
        &[("``", "``", "string"), ("**", "**", "attribute"), ("*", "*", "attribute")]
    } else {
        &[
            ("``", "``", "string"),
            ("`", "`", "string"),
            ("**", "**", "attribute"),
            ("__", "__", "attribute"),
            ("*", "*", "attribute"),
            ("_", "_", "attribute"),
        ]
    };
    for (open, close, role) in pairs {
        if source[i..].starts_with(open) {
            if let Some(end) = paired(source, i, open, close) {
                return Some((end, role));
            }
        }
    }
    if d.has(MD) && bytes[i] == b'(' && i > 0 && bytes[i - 1] == b']' {
        return paired(source, i, "(", ")").map(|end| (end, "string"));
    }
    None
}

/// Quoted, triple-quoted, prefixed and raw strings; returns the byte index past the close.
fn string_at(source: &str, i: usize, d: &Dialect) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut j = i;
    while j < bytes.len() && d.raw.as_bytes().contains(&bytes[j]) && bytes[j] != b'#' {
        j += 1;
    }
    let hashes = if d.raw.contains('#') {
        bytes[j..].iter().take_while(|&&c| c == b'#').count()
    } else {
        0
    };
    let quote_at = j + hashes;
    let quote = *bytes.get(quote_at)?;
    if !d.quotes.as_bytes().contains(&quote) {
        return None;
    }
    let raw = quote_at > i;
    if quote == b'\'' && d.has(CHAR) && !char_literal(&source[quote_at..]) {
        return None;
    }
    let triple = d.has(TRIPLE) && bytes[quote_at..].starts_with(&[quote; 3]);
    let mut k = quote_at + if triple { 3 } else { 1 };
    while k < bytes.len() {
        if triple {
            if bytes[k..].starts_with(&[quote; 3]) {
                return Some(k + 3);
            }
            k += 1;
        } else if hashes > 0 {
            if bytes[k] == quote
                && bytes
                    .get(k + 1..k + 1 + hashes)
                    .is_some_and(|v| v.iter().all(|&c| c == b'#'))
            {
                return Some(k + 1 + hashes);
            }
            k += 1;
        } else if bytes[k] == b'\\' && !raw {
            k += 2;
        } else if bytes[k] == quote {
            return Some(k + 1);
        } else if bytes[k] == b'\n' {
            return Some(k);
        } else {
            k += 1;
        }
    }
    Some(bytes.len())
}

// ---- detection ----

fn any_line(src: &str, prefix: &str) -> bool {
    src.lines().any(|l| l.trim_start().starts_with(prefix))
}
fn col0(src: &str, prefix: &str) -> bool {
    src.lines().any(|l| l.starts_with(prefix))
}

fn signal(t: &str, src: &str) -> Option<&'static str> {
    if let Some(rest) = t.strip_prefix("#!") {
        let line = rest.lines().next().unwrap_or("");
        for (needle, name) in [
            ("python", "python"),
            ("perl", "perl"),
            ("ruby", "ruby"),
            ("node", "javascript"),
            ("deno", "typescript"),
            ("tclsh", "tcl"),
            ("wish", "tcl"),
            ("lua", "lua"),
            ("Rscript", "r"),
            ("groovy", "groovy"),
            ("php", "php"),
            ("pwsh", "powershell"),
        ] {
            if line.contains(needle) {
                return Some(name);
            }
        }
        return Some("shell");
    }
    let low = t.to_ascii_lowercase();
    if low.contains("<?php") {
        return Some("php");
    }
    if low.starts_with("<?xml") {
        return Some("xml");
    }
    if low.starts_with("<!doctype html") || low.contains("<html") || low.contains("<div") || low.contains("<body") {
        return Some("html");
    }
    if t.contains("<%") {
        return Some(if low.contains("dim ") || low.contains("response.") { "asp" } else { "erb" });
    }
    if low.starts_with('<') && low.contains("</") {
        return Some("xml");
    }
    if t.starts_with("diff --git") || t.starts_with("Index:") || src.contains("\n@@ ") || t.starts_with("@@ ")
        || t.starts_with("--- ") && src.contains("\n+++ ")
    {
        return Some("diff");
    }
    for section in ["[core]", "[remote \"", "[branch \"", "[alias]", "[user]"] {
        if src.contains(section) {
            return Some("git");
        }
    }
    if t.starts_with("\\documentclass") || t.contains("\\begin{") || t.contains("\\section{") || t.contains("\\usepackage") {
        return Some("latex");
    }
    if (t.starts_with('{') || t.starts_with('[')) && json_like(t) {
        return Some("json");
    }
    if src.contains("@interface") || src.contains("@implementation") || src.contains("#import <") || src.contains("NSString") {
        return Some("objective-c");
    }
    if src.contains("#include") {
        let plus = ["std::", "template", "namespace", "class ", "public:", "cout", "nullptr"];
        return Some(if plus.iter().any(|p| src.contains(p)) { "cpp" } else { "c" });
    }
    if src.contains("using System") {
        return Some("csharp");
    }
    if any_line(src, "package ") && (src.contains("func ") || src.contains("import (")) {
        return Some("go");
    }
    if src.contains("#[derive") || src.contains("fn main") || src.contains("let mut ") || src.contains("impl ") || src.contains("::<") {
        return Some("rust");
    }
    if src.contains("fun ") && (src.contains("val ") || src.contains("var ")) {
        return Some("kotlin");
    }
    if src.contains("func ") && (src.contains("guard ") || src.contains("import Foundation") || src.contains("let ") && src.contains("-> ")) {
        return Some("swift");
    }
    if src.contains("import java") || src.contains("public class") || src.contains("public static void main") {
        return Some("java");
    }
    if src.contains("def ") && (col0(src, "end") || any_line(src, "end\n") || src.contains("\nend") || src.contains("puts ")) && !src.contains("):") {
        return Some("ruby");
    }
    if any_line(src, "def ") && src.contains(':') || src.contains("if __name__") || any_line(src, "import ") && src.contains("print(") {
        return Some("python");
    }
    if src.contains("local ") && (src.contains("function") || src.contains("end")) {
        return Some("lua");
    }
    if src.contains("<-") && (src.contains("function") || src.contains("library(")) {
        return Some("r");
    }
    let curly = ["function ", "=>", "const ", "let ", "var "].iter().any(|p| src.contains(p));
    if curly && (src.contains('{') || src.contains(';')) {
        let typed = [": string", ": number", ": boolean", "interface ", "as const", "readonly "]
            .iter()
            .any(|p| src.contains(p));
        return Some(if typed { "typescript" } else { "javascript" });
    }
    let sql = low.contains("select ") && low.contains(" from ")
        || low.contains("insert into")
        || low.contains("create table")
        || low.contains("update ") && low.contains(" set ");
    if sql {
        return Some("sql");
    }
    if low.starts_with("@echo off") || src.contains("%~dp0") {
        return Some("batch");
    }
    if src.contains("$PSScriptRoot") || src.contains("Write-Host") || src.contains("param(") && src.contains('$') {
        return Some("powershell");
    }
    if src.contains("digraph ") || src.contains("graph {") || src.contains(" -> ") && src.contains("label=") {
        return Some("dot");
    }
    if src.contains("tell application") || src.contains("end tell") || src.contains("display dialog") {
        return Some("applescript");
    }
    if t.starts_with('(') && (src.contains("defn ") || src.contains("(ns ") || src.contains("#{")) {
        return Some("clojure");
    }
    if t.starts_with('(') && (src.contains("defun ") || src.contains("(define ") || src.contains("setq ")) {
        return Some("lisp");
    }
    if src.contains(" :: ") && src.contains("->") && (src.contains("module ") || src.contains("where")) {
        return Some("haskell");
    }
    if col0(src, ".PHONY") || makefile_like(src) {
        return Some("makefile");
    }
    if toml_like(src) {
        return Some("toml");
    }
    if yaml_like(t, src) {
        return Some("yaml");
    }
    if css_like(src) {
        return Some("css");
    }
    if any_line(src, "h1. ") || any_line(src, "h2. ") {
        return Some("textile");
    }
    if src.contains(".. code-block::") || src.lines().any(|l| rule_line(l) && l.trim_end().len() >= 4) && src.contains("\n\n") {
        return Some("rst");
    }
    if any_line(src, "# ") && (src.contains("](") || src.contains("**") || src.contains("```") || any_line(src, "- ")) {
        return Some("markdown");
    }
    if src.contains("fi\n") || src.contains("esac") || src.contains("echo ") && src.contains("$(") {
        return Some("shell");
    }
    None
}

fn json_like(t: &str) -> bool {
    let end = t.trim_end();
    (end.ends_with('}') || end.ends_with(']'))
        && t.contains('"')
        && t.contains(':')
        && !t.contains("=>")
        && !t.contains(';')
        && !t.contains("//")
        && !t.contains("function")
}
fn makefile_like(src: &str) -> bool {
    let lines: Vec<_> = src.lines().collect();
    lines.windows(2).any(|w| {
        let target = w[0]
            .split_once(':')
            .is_some_and(|(name, _)| !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-./%$() ".contains(&c)));
        target && !w[0].starts_with(char::is_whitespace) && w[1].starts_with('\t')
    })
}
fn toml_like(src: &str) -> bool {
    src.lines().any(|l| l.starts_with('[') && l.trim_end().ends_with(']'))
        && src.lines().filter(|l| l.contains(" = ")).count() >= 1
}
fn yaml_like(t: &str, src: &str) -> bool {
    if src.contains('{') || src.contains(';') {
        return false;
    }
    let keys = src
        .lines()
        .filter(|l| {
            let l = l.trim_start().trim_start_matches("- ");
            l.split_once(':')
                .is_some_and(|(k, v)| !k.is_empty() && k.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c)) && (v.is_empty() || v.starts_with(' ')))
        })
        .count();
    t.starts_with("---") && keys >= 1 || keys >= 2
}
fn css_like(src: &str) -> bool {
    src.contains('{')
        && src.contains('}')
        && src.lines().any(|l| {
            let l = l.trim();
            l.ends_with(';')
                && l.split_once(':')
                    .is_some_and(|(k, _)| !k.is_empty() && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
        })
        && src.lines().any(|l| l.trim_end().ends_with('{'))
}

/// Running prose: words with next to no code punctuation. Keyword tables are full of English
/// (`set`, `is`, `not`, `run`, `It`), so scoring an address or a log line invents a language.
fn prose(src: &str) -> bool {
    let mut words = 0usize;
    let mut marks = 0usize;
    let mut inside = false;
    for ch in src.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            words += usize::from(!inside);
            inside = true;
        } else {
            inside = false;
            marks += usize::from("{}[]()<>;=|&*+/\\%$#@^~`".contains(ch));
        }
    }
    words >= 6 && marks * 8 < words
}

/// Inverse-frequency keyword score; a dialect wins only with a clear margin over the runner-up.
fn score(src: &str) -> Option<&'static str> {
    if prose(src) {
        return None;
    }
    let mut words: Vec<&str> = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() && words.len() < 96 {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            if word.len() >= 2 && !words.contains(&word) {
                words.push(word);
            }
        } else {
            i += 1;
        }
    }
    let count = dialects::DIALECTS.len();
    let mut totals = vec![0f32; count];
    let mut hits = vec![0u32; count];
    let mut total = 0f32;
    let mut matched = 0;
    for word in &words {
        let mut mask = 0u64;
        for (k, d) in dialects::DIALECTS.iter().enumerate() {
            if d.known(word) {
                mask |= 1 << k;
            }
        }
        if mask == 0 {
            continue;
        }
        matched += 1;
        let weight = 1.0 / mask.count_ones() as f32;
        total += weight;
        for (k, slot) in totals.iter_mut().enumerate() {
            if mask & (1 << k) != 0 {
                *slot += weight;
                hits[k] += 1;
            }
        }
    }
    // Three distinct hits at least: one or two rare words are a coincidence, not a language.
    if matched < 3 || total <= 0.0 {
        return None;
    }
    let mut best = (0usize, 0f32);
    let mut second = 0f32;
    for (k, value) in totals.iter().copied().enumerate() {
        if value > best.1 {
            second = best.1;
            best = (k, value);
        } else if value > second {
            second = value;
        }
    }
    (hits[best.0] >= 3 && best.1 >= 0.5 * total && best.1 >= 1.8 * second)
        .then(|| dialects::DIALECTS[best.0].name)
}
