use crate::code::{detect, highlight, Token};

fn roles(language: &str, source: &str) -> Vec<(String, &'static str)> {
    let units: Vec<u16> = source.encode_utf16().collect();
    highlight(source, language, 0)
        .iter()
        .map(|t| {
            (
                String::from_utf16(&units[t.start as usize..t.end as usize]).unwrap(),
                t.role,
            )
        })
        .collect()
}

/// (language, snippet, expected (text, role) pairs).
const SNIPPETS: &[(&str, &str, &[(&str, &str)])] = &[
    (
        "rust",
        "// tally\nfn main() {\n    let total: u32 = 41;\n    println!(\"{}\", total);\n}\n",
        &[("// tally", "comment"), ("fn", "keyword"), ("u32", "type"), ("41", "number"), ("\"{}\"", "string"), ("println", "function")],
    ),
    (
        "kotlin",
        "// tally\nfun main() {\n    val total: Int = 41\n    println(\"hi\")\n}\n",
        &[("// tally", "comment"), ("fun", "keyword"), ("Int", "type"), ("41", "number"), ("\"hi\"", "string")],
    ),
    (
        "c",
        "#include <stdio.h>\nint main(void) {\n    int total = 41; /* count */\n    return 0;\n}\n",
        &[("#include <stdio.h>", "preprocessor"), ("int", "type"), ("41", "number"), ("/* count */", "comment"), ("return", "keyword")],
    ),
    (
        "cpp",
        "#include <vector>\nint main() {\n    std::vector<int> v = {4, 2};  // pair\n    return 0;\n}\n",
        &[("#include <vector>", "preprocessor"), ("vector", "type"), ("// pair", "comment"), ("4", "number"), ("return", "keyword")],
    ),
    (
        "csharp",
        "using System;\nclass Counter {\n    public int Total = 41; // count\n    public string Name = \"unit\";\n}\n",
        &[("using", "keyword"), ("int", "type"), ("41", "number"), ("// count", "comment"), ("\"unit\"", "string")],
    ),
    (
        "java",
        "// tally\npublic class Counter {\n    private int total = 41;\n    private String name = \"unit\";\n}\n",
        &[("public", "keyword"), ("int", "type"), ("41", "number"), ("\"unit\"", "string"), ("// tally", "comment")],
    ),
    (
        "javascript",
        "// tally\nconst total = 41;\nfunction greet(name) {\n    return `hi ${name}`;\n}\n",
        &[("const", "keyword"), ("41", "number"), ("greet", "function"), ("// tally", "comment"), ("`hi ${name}`", "string")],
    ),
    (
        "typescript",
        "// tally\nconst total: number = 41;\nfunction greet(name: string): string {\n    return \"hi \" + name;\n}\n",
        &[("const", "keyword"), ("number", "type"), ("41", "number"), ("\"hi \"", "string")],
    ),
    (
        "python",
        "# tally\ndef greet(name: str) -> str:\n    total = 41\n    return f\"hi {name}\"\n",
        &[("# tally", "comment"), ("def", "keyword"), ("str", "type"), ("41", "number"), ("f\"hi {name}\"", "string")],
    ),
    (
        "ruby",
        "# tally\ndef greet(name)\n  total = 41\n  puts \"hi\"\nend\n",
        &[("# tally", "comment"), ("def", "keyword"), ("41", "number"), ("\"hi\"", "string"), ("end", "keyword")],
    ),
    (
        "erb",
        "<div class=\"row\">\n  <% 3.times do |n| %>\n    <span><%= n %></span>\n  <% end %>\n</div>\n",
        &[("<div", "tag"), ("class", "attribute"), ("\"row\"", "string"), ("<%", "tag"), ("3", "number"), ("do", "keyword"), ("end", "keyword")],
    ),
    (
        "php",
        "<?php\n// tally\n$total = 41;\necho \"hi\";\n?>\n",
        &[("<?php", "tag"), ("// tally", "comment"), ("$total", "variable"), ("41", "number"), ("echo", "keyword"), ("\"hi\"", "string")],
    ),
    (
        "go",
        "package main\n\nimport \"fmt\"\n\nfunc main() { // tally\n    total := 41\n    fmt.Println(total)\n}\n",
        &[("package", "keyword"), ("\"fmt\"", "string"), ("41", "number"), ("// tally", "comment"), ("func", "keyword")],
    ),
    (
        "swift",
        "// tally\nfunc greet() -> String {\n    let total = 41\n    return \"hi\"\n}\n",
        &[("func", "keyword"), ("String", "type"), ("41", "number"), ("\"hi\"", "string"), ("// tally", "comment")],
    ),
    (
        "dart",
        "// tally\nvoid main() {\n  int total = 41;\n  print('hi');\n}\n",
        &[("void", "type"), ("int", "type"), ("41", "number"), ("'hi'", "string"), ("// tally", "comment")],
    ),
    (
        "scala",
        "// tally\nobject Counter {\n  val total: Int = 41\n  def name: String = \"unit\"\n}\n",
        &[("object", "keyword"), ("Int", "type"), ("41", "number"), ("\"unit\"", "string")],
    ),
    (
        "groovy",
        "// tally\ndef total = 41\ndef greet(String name) {\n    println \"hi\"\n}\n",
        &[("def", "keyword"), ("41", "number"), ("String", "type"), ("\"hi\"", "string"), ("// tally", "comment")],
    ),
    (
        "haskell",
        "-- tally\ntotal :: Int\ntotal = 41\n\ngreet :: String -> String\ngreet name = \"hi \" ++ name\n",
        &[("-- tally", "comment"), ("Int", "type"), ("41", "number"), ("\"hi \"", "string")],
    ),
    (
        "ocaml",
        "(* tally *)\nlet total = 41\nlet greet name = \"hi \" ^ name\n",
        &[("(* tally *)", "comment"), ("let", "keyword"), ("41", "number"), ("\"hi \"", "string")],
    ),
    (
        "lisp",
        ";; tally\n(defun greet (name)\n  (let ((total 41))\n    (format nil \"hi ~a\" name)))\n",
        &[(";; tally", "comment"), ("defun", "keyword"), ("41", "number"), ("\"hi ~a\"", "string"), ("nil", "constant")],
    ),
    (
        "clojure",
        ";; tally\n(defn greet [name]\n  (let [total 41]\n    (str \"hi \" name)))\n",
        &[(";; tally", "comment"), ("defn", "keyword"), ("41", "number"), ("\"hi \"", "string"), ("str", "function")],
    ),
    (
        "lua",
        "-- tally\nlocal total = 41\nfunction greet(name)\n  return \"hi \" .. name\nend\n",
        &[("-- tally", "comment"), ("local", "keyword"), ("41", "number"), ("\"hi \"", "string"), ("end", "keyword")],
    ),
    (
        "perl",
        "# tally\nmy $total = 41;\nsub greet {\n    my ($name) = @_;\n    print \"hi\";\n}\n",
        &[("# tally", "comment"), ("my", "keyword"), ("$total", "variable"), ("41", "number"), ("@_", "variable"), ("\"hi\"", "string")],
    ),
    (
        "r",
        "# tally\ntotal <- 41\ngreet <- function(name) {\n  paste(\"hi\", name)\n}\n",
        &[("# tally", "comment"), ("41", "number"), ("function", "keyword"), ("\"hi\"", "string")],
    ),
    (
        "matlab",
        "% tally\nfunction out = greet(name)\n    total = 41;\n    out = sprintf('hi %s', name);\nend\n",
        &[("% tally", "comment"), ("function", "keyword"), ("41", "number"), ("'hi %s'", "string"), ("end", "keyword")],
    ),
    (
        "sql",
        "-- tally\nSELECT name, total\nFROM counters\nWHERE total > 41 AND name = 'unit';\n",
        &[("-- tally", "comment"), ("SELECT", "keyword"), ("FROM", "keyword"), ("41", "number"), ("'unit'", "string")],
    ),
    (
        "shell",
        "# tally\ntotal=41\ngreet() {\n  echo \"hi $1\"\n}\n",
        &[("# tally", "comment"), ("41", "number"), ("echo", "keyword"), ("\"hi $1\"", "string")],
    ),
    (
        "powershell",
        "# tally\nfunction Total {\n    $total = 41\n    Write \"hi $total\"\n}\n",
        &[("# tally", "comment"), ("function", "keyword"), ("$total", "variable"), ("41", "number"), ("\"hi $total\"", "string")],
    ),
    (
        "batch",
        "@echo off\nrem tally\nset TOTAL=41\necho hi %TOTAL%\n",
        &[("rem tally", "comment"), ("echo", "keyword"), ("set", "keyword"), ("41", "number"), ("%TOTAL%", "variable")],
    ),
    (
        "tcl",
        "# tally\nproc greet {name} {\n    set total 41\n    puts \"hi $name\"\n}\n",
        &[("# tally", "comment"), ("proc", "keyword"), ("set", "keyword"), ("41", "number"), ("\"hi $name\"", "string")],
    ),
    (
        "makefile",
        "# tally\nVERSION = 41\nall: build\n\t$(CC) -o app main.c\n",
        &[("# tally", "comment"), ("VERSION", "key"), ("41", "number"), ("all", "key"), ("$(CC)", "variable")],
    ),
    (
        "json",
        "{\n  \"name\": \"unit\",\n  \"total\": 41,\n  \"ready\": true\n}\n",
        &[("\"name\"", "key"), ("\"unit\"", "string"), ("41", "number"), ("true", "constant")],
    ),
    (
        "yaml",
        "# tally\nname: unit\ntotal: 41\ntags:\n  - alpha\nready: true\n",
        &[("# tally", "comment"), ("name", "key"), ("41", "number"), ("true", "constant")],
    ),
    (
        "toml",
        "# tally\n[package]\nname = \"unit\"\ntotal = 41\nready = true\n",
        &[("# tally", "comment"), ("[package]", "heading"), ("name", "key"), ("\"unit\"", "string"), ("41", "number"), ("true", "constant")],
    ),
    (
        "git",
        "# tally\n[core]\n\teditor = vim\n\tcompression = 9\n",
        &[("# tally", "comment"), ("[core]", "heading"), ("editor", "key"), ("9", "number")],
    ),
    (
        "diff",
        "--- a/main.c\n+++ b/main.c\n@@ -1,4 +1,4 @@\n-int total = 40;\n+int total = 41;\n return 0;\n",
        &[("--- a/main.c", "heading"), ("+++ b/main.c", "heading"), ("@@ -1,4 +1,4 @@", "heading"), ("-int total = 40;", "deleted"), ("+int total = 41;", "inserted")],
    ),
    (
        "dot",
        "// tally\ndigraph flow {\n  rankdir = LR;\n  weight = 41;\n}\n",
        &[("// tally", "comment"), ("digraph", "keyword"), ("rankdir", "key"), ("41", "number")],
    ),
    (
        "css",
        "/* tally */\n@media screen {\n  .card {\n    color: red;\n    margin: 41px;\n  }\n}\n",
        &[("/* tally */", "comment"), ("@media", "keyword"), ("color", "key"), ("41px", "number")],
    ),
    (
        "html",
        "<!-- tally -->\n<div class=\"card\">\n  <p>total 41</p>\n</div>\n",
        &[("<!-- tally -->", "comment"), ("<div", "tag"), ("class", "attribute"), ("\"card\"", "string"), ("41", "number")],
    ),
    (
        "xml",
        "<?xml version=\"1.0\"?>\n<!-- tally -->\n<items count=\"2\">\n  <item name=\"unit\">41</item>\n</items>\n",
        &[("<?xml", "tag"), ("<!-- tally -->", "comment"), ("count", "attribute"), ("\"unit\"", "string"), ("41", "number")],
    ),
    (
        "markdown",
        "# Tally\n\nA **bold** word and `code` with 41 items.\n\n- alpha\n",
        &[("# Tally", "heading"), ("**bold**", "attribute"), ("`code`", "string"), ("41", "number")],
    ),
    (
        "rst",
        "Tally\n=====\n\nA ``literal`` and *emphasis* with 41 items.\n\n.. note:: done\n",
        &[("=====", "heading"), ("``literal``", "string"), ("*emphasis*", "attribute"), ("41", "number"), (".. note:: done", "preprocessor")],
    ),
    (
        "textile",
        "h1. Tally\n\nA *bold* word and @code@ with 41 items.\n",
        &[("h1. Tally", "heading"), ("*bold*", "attribute"), ("@code@", "string"), ("41", "number")],
    ),
    (
        "latex",
        "% tally\n\\documentclass{article}\n\\begin{document}\nTotal 41 items.\n\\end{document}\n",
        &[("% tally", "comment"), ("\\documentclass", "keyword"), ("\\begin", "keyword"), ("{document}", "type"), ("41", "number")],
    ),
    (
        "regex",
        "^(?:[a-z]+)\\d{2,4}$",
        &[("^", "operator"), ("(?:", "punctuation"), ("[a-z]", "type"), ("+", "operator"), ("\\d", "constant"), ("{2,4}", "operator"), ("$", "operator")],
    ),
    (
        "pascal",
        "{ tally }\nprogram Counter;\nvar\n  Total: Integer;\nbegin\n  Total := 41;\n  WriteLn('hi');\nend.\n",
        &[("{ tally }", "comment"), ("program", "keyword"), ("Integer", "type"), ("41", "number"), ("'hi'", "string")],
    ),
    (
        "objective-c",
        "#import <Foundation/Foundation.h>\n// tally\nstatic const NSInteger kMax = 41;\n@interface Counter : NSObject\n@property (nonatomic) NSInteger total;\n@end\n",
        &[("#import <Foundation/Foundation.h>", "preprocessor"), ("// tally", "comment"), ("@interface", "keyword"), ("NSObject", "type"), ("NSInteger", "type"), ("41", "number")],
    ),
    (
        "d",
        "// tally\nimport std.stdio;\nvoid main() {\n    int total = 41;\n    writeln(\"hi\");\n}\n",
        &[("// tally", "comment"), ("import", "keyword"), ("int", "type"), ("41", "number"), ("\"hi\"", "string")],
    ),
    (
        "actionscript",
        "// tally\npackage {\n    public class Counter {\n        public var total:int = 41;\n        public var name:String = \"unit\";\n    }\n}\n",
        &[("package", "keyword"), ("int", "type"), ("41", "number"), ("\"unit\"", "string"), ("// tally", "comment")],
    ),
    (
        "applescript",
        "-- tally\nset total to 41\ntell application \"Finder\"\n    display dialog \"hi\"\nend tell\n",
        &[("-- tally", "comment"), ("set", "keyword"), ("41", "number"), ("\"Finder\"", "string"), ("tell", "keyword")],
    ),
    (
        "asp",
        "<%\n' tally\nDim total\ntotal = 41\nResponse.Write \"hi\"\n%>\n",
        &[("<%", "tag"), ("' tally", "comment"), ("Dim", "keyword"), ("41", "number"), ("\"hi\"", "string")],
    ),
];

#[test]
fn every_dialect_tokenises_its_snippet() {
    assert_eq!(SNIPPETS.len(), 51);
    let mut missing = Vec::new();
    for (language, source, expected) in SNIPPETS {
        let found = roles(language, source);
        for (text, role) in *expected {
            if !found.iter().any(|(t, r)| t == text && r == role) {
                missing.push(format!("{language}: expected {text:?} as {role}, got {found:?}"));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn aliases_resolve_and_stay_unique() {
    assert!(crate::dialects::DIALECTS.len() <= 64);
    let mut seen: Vec<&str> = Vec::new();
    for d in crate::dialects::DIALECTS {
        assert!(d.aliases.split_ascii_whitespace().any(|a| a == d.name), "{} misses its own name", d.name);
        for alias in d.aliases.split_ascii_whitespace() {
            assert!(!seen.contains(&alias), "duplicate alias {alias}");
            seen.push(alias);
        }
    }
    for (alias, name) in [("rs", "rust"), ("PY", "python"), ("c++", "cpp"), ("yml", "yaml"), ("bash", "shell"), ("objc", "objective-c")] {
        assert_eq!(crate::dialects::find(alias).map(|d| d.name), Some(name));
    }
    assert!(crate::dialects::find("nonesuch").is_none());
    assert!(highlight("let x = 1;", "nonesuch", 0).is_empty());
}

/// (source, expected detection) — including pairs that must not be confused.
const DETECTION: &[(&str, Option<&str>)] = &[
    ("fn main() {\n    let mut total = 41;\n    println!(\"{}\", total);\n}\n", Some("rust")),
    ("def greet(name):\n    total = 41\n    return f\"hi {name}\"\n", Some("python")),
    ("def greet(name)\n  total = 41\n  puts \"hi\"\nend\n", Some("ruby")),
    ("#include <stdio.h>\nint main(void) { return 0; }\n", Some("c")),
    ("#include <vector>\nstd::vector<int> v;\nnamespace app { }\n", Some("cpp")),
    ("import java.util.List;\npublic class Counter {\n  private int total = 41;\n}\n", Some("java")),
    ("{\n  \"name\": \"unit\",\n  \"total\": 41\n}\n", Some("json")),
    ("const total = 41;\nfunction greet(name) {\n  return `hi ${name}`;\n}\n", Some("javascript")),
    ("name: unit\ntotal: 41\nready: true\n", Some("yaml")),
    ("[package]\nname = \"unit\"\nversion = \"0.1.0\"\n", Some("toml")),
    ("<!DOCTYPE html>\n<html>\n<body><p>hi</p></body>\n</html>\n", Some("html")),
    ("<?xml version=\"1.0\"?>\n<items><item>41</item></items>\n", Some("xml")),
    ("package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(41) }\n", Some("go")),
    ("fun main() {\n    val total = 41\n    println(total)\n}\n", Some("kotlin")),
    ("import Foundation\nfunc greet() -> String {\n    let total = 41\n    return \"hi\"\n}\n", Some("swift")),
    ("<?php\n$total = 41;\necho \"hi\";\n", Some("php")),
    ("SELECT name FROM counters WHERE total > 41;\n", Some("sql")),
    ("#!/bin/bash\nset -eu\necho \"hi\"\n", Some("shell")),
    ("--- a/main.c\n+++ b/main.c\n@@ -1 +1 @@\n-old\n+new\n", Some("diff")),
    ("all: build\n\tgcc -o app main.c\n\nclean:\n\trm -f app\n", Some("makefile")),
    ("\\documentclass{article}\n\\begin{document}\nhi\n\\end{document}\n", Some("latex")),
    (".card {\n  color: red;\n  margin: 4px;\n}\n", Some("css")),
    ("# Tally\n\nSome **bold** text and a [link](https://example.invalid).\n", Some("markdown")),
    ("using System;\nnamespace App { class Counter { } }\n", Some("csharp")),
    ("#import <Foundation/Foundation.h>\n@interface Counter : NSObject\n@end\n", Some("objective-c")),
    ("param($name)\nWrite-Host \"hi $name\"\n", Some("powershell")),
    ("@echo off\nset TOTAL=41\necho %TOTAL%\n", Some("batch")),
    ("(ns app.core)\n(defn greet [name] (str \"hi \" name))\n", Some("clojure")),
    ("(defun greet (name)\n  (format nil \"hi ~a\" name))\n", Some("lisp")),
    ("module Main where\n\ngreet :: String -> String\ngreet name = \"hi \"\n", Some("haskell")),
    ("digraph flow {\n  a -> b;\n}\n", Some("dot")),
    ("[core]\n\teditor = vim\n[branch \"main\"]\n\tremote = origin\n", Some("git")),
    ("#!/usr/bin/env python3\nprint(41)\n", Some("python")),
    ("#!/usr/bin/perl\nmy $total = 41;\n", Some("perl")),
    ("-- tally\nlocal total = 41\nfunction greet() return \"hi\" end\n", Some("lua")),
    ("the quick brown fox jumped over it\n", None),
    ("", None),
    ("41 42 43\n", None),
    // Ordinary text must stay unlabelled: keyword tables are full of English.
    ("Unit 4, 128 Example Street\nSome Town, 12345\nCountry\n", None),
    ("Traceback (most recent call last):\n  File \"a.py\", line 3, in <module>\n    main()\nValueError: bad\n", None),
    ("Exception in thread \"main\" java.lang.NullPointerException\n\tat com.example.A.run(A.java:12)\n\tat com.example.A.main(A.java:4)\n", None),
    ("2026-01-02 12:00:01 INFO  starting up\n2026-01-02 12:00:02 WARN  disk almost full\n2026-01-02 12:00:03 ERROR could not write\n", None),
    ("Dear Sir or Madam,\n\nThank you for your letter of the third.\nI will reply in due course.\n", None),
    ("Total due: 128.50\nPaid in full on the third of January.\nThank you for your custom.\n", None),
    ("The set of all sets is not a set. Consider the case where it were.\n", None),
    ("if you want to go there, take the second left and then turn right\n", None),
];

#[test]
fn detection_separates_the_near_misses() {
    assert!(DETECTION.iter().filter(|(_, want)| want.is_some()).count() >= 20);
    let mut wrong = Vec::new();
    for (source, want) in DETECTION {
        let got = detect(source);
        if got != *want {
            wrong.push(format!("{source:?}: wanted {want:?}, got {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn detection_runs_at_presentation_only_and_reports_the_language() {
    let text = crate::parse("```\nfn main() { let mut n = 1; }\n```", Default::default()).unwrap();
    assert_eq!(
        text.blocks()[0].kind,
        crate::BlockKind::Code { language: None },
        "the canonical block keeps no language"
    );
    let view = text.presentation();
    assert_eq!(
        view.blocks[0].kind,
        crate::BlockKind::Code {
            language: Some("rust".into())
        }
    );
    assert!(view.code_tokens.iter().any(|t| t.role == "keyword"));
    let bytes = text.to_bytes().unwrap();
    assert!(!std::str::from_utf8(&bytes).unwrap().contains("rust"));
    // An explicit tag always wins over detection.
    let tagged = crate::parse("```json\nfn main() {}\n```", Default::default()).unwrap();
    assert_eq!(
        tagged.presentation().blocks[0].kind,
        crate::BlockKind::Code {
            language: Some("json".into())
        }
    );
}

#[test]
fn presentation_tokens_use_utf16_and_do_not_enter_the_canonical_message() {
    let text = crate::parse("👋\n\n```rust\nlet code = \"👩🏽‍💻\";\n```", Default::default()).unwrap();
    let bytes = text.to_bytes().unwrap();
    let view = text.presentation();
    let units: Vec<_> = view.text.encode_utf16().collect();
    let slice = |t: &Token| String::from_utf16(&units[t.start as usize..t.end as usize]).unwrap();
    let of = |role: &str| view.code_tokens.iter().filter(|t| t.role == role).map(slice).collect::<Vec<_>>();
    assert_eq!(of("keyword"), ["let"]);
    assert_eq!(of("string"), ["\"👩🏽‍💻\""]);
    assert!(!std::str::from_utf8(&bytes).unwrap().contains("code_tokens"));
    assert_eq!(crate::Text::from_bytes(&bytes).unwrap().to_bytes().unwrap(), bytes);
}

#[test]
fn highlighting_preserves_unicode_literals_lifetimes_and_unknown_languages() {
    let source = "let glyph = r##\"👩🏽‍💻 // literal\"##; /* outer /* inner */ end */ &'static str";
    let tokens: Vec<Token> = highlight(source, "rust", 4);
    let utf16: Vec<_> = source.encode_utf16().collect();
    let text = |token: &Token| String::from_utf16(&utf16[(token.start - 4) as usize..(token.end - 4) as usize]).unwrap();
    let of = |role: &str| tokens.iter().filter(|t| t.role == role).map(text).collect::<Vec<_>>();
    assert_eq!(of("keyword"), ["let", "static"]);
    assert_eq!(of("string"), ["r##\"👩🏽‍💻 // literal\"##"]);
    assert_eq!(of("comment"), ["/* outer /* inner */ end */"]);
    assert!(highlight(source, "unknown", 0).is_empty());
    for input in ["\"escaped\\👋\"", "'👋'", "\"unterminated\\", "r#identifier", "«", "🙂 🙂"] {
        for (language, _, _) in SNIPPETS {
            highlight(input, language, 0);
        }
    }
    assert_eq!(highlight("true false null", "json", 0).len(), 3);
}


#[test]
fn the_fixed_role_set_reaches_every_dialect_and_data_formats_invent_no_calls() {
    let rust = roles("rust", "let n = f(1) + 2;");
    assert!(rust.contains(&("+".to_string(), "operator")) && rust.contains(&(";".to_string(), "punctuation")));
    assert!(rust.contains(&("f".to_string(), "function")));
    // A data format has no calls: `main` before `(` is not a function there.
    let json = roles("json", "fn main() {\n  \"a\": 1\n}");
    assert!(!json.iter().any(|(_, role)| *role == "function"));
    assert!(json.contains(&("{".to_string(), "punctuation")));
    // Prose-bodied dialects stay unmarked.
    for language in ["markdown", "diff", "html", "latex", "rst", "textile"] {
        assert!(!roles(language, "a - b; (c)").iter().any(|(_, r)| *r == "operator"));
    }
}

#[test]
fn hyphenated_and_suffixed_keywords_are_not_dead_table_entries() {
    for (language, source, word) in [
        ("clojure", "(defn- f [] (if-let [x 1] (when-not x (set! y 2))))", "if-let"),
        ("clojure", "(defn- f [])", "defn-"),
        ("lisp", "(let* ((a 1)) (set! a 2))", "let*"),
        ("lisp", "(set! a 2)", "set!"),
        ("css", "font-face { color: red; }", "font-face"),
    ] {
        assert!(
            roles(language, source).contains(&(word.to_string(), "keyword")),
            "{language} missed {word}"
        );
    }
}

#[test]
fn a_plain_text_fence_reports_no_language_and_no_colour() {
    for tag in ["text", "txt", "Plain", "plaintext", "none"] {
        let text = crate::parse(&format!("```{tag}\nfn main() {{}}\n```"), Default::default()).unwrap();
        let view = text.presentation();
        assert_eq!(view.blocks[0].kind, crate::BlockKind::Code { language: None }, "{tag}");
        assert!(view.code_tokens.is_empty(), "{tag}");
    }
    assert!(crate::builder::code_preview("Code\ntext\nfn main() {}").unwrap().starts_with("\n\n"));
}
