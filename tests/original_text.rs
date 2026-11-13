//! The games' own text stays out of the code: every word the engine shows of a game's own comes from its converted
//! text table (`assets::text`). Data-gated: with the text tables of the test data (`OKB_EXPORT` or `../okb_export`,
//! `BR1_EXPORT` or `../okb_export/br1`; skipped without them) it fails on
//! - a string literal in the code of `src/` or `tests/` (test code too; not in comments) that is one of their texts
//!   of two characters or more, spaces at the ends and runs of spaces aside, as written or with its numbers as a
//!   value mark; a format string's `{…}` matches a text's value mark (`%s`, `%u`, `%d`). `ALLOWED` lists the
//!   exceptions (a test's expectation takes the text from the text API instead);
//! - a key the code names literally (`text::get("…")`, `text::item("…", …)`, `text::br1("…")`, `text::br1_item("…",
//!   …)`, `Label::Game("…")`) that the game's table lacks (a list's key: its entry 0).

mod common;
use common::*;
use std::collections::BTreeSet;
use std::path::Path;

/// (file under `src/` or `tests/`, literal, why) the check lets pass.
const ALLOWED: &[(&str, &str, &str)] = &[
    // The imported game's fighter and movie codes: file stems that are also names
    ("assets/manifest.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("assets/manifest.rs", "GREG", "the imported game's movie file stem"),
    ("assets/roster.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("fight/stage/br1.rs", "FOX", "the imported game's stage folder and code stem"),
    ("formats/br1_anm.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("frontend/movie.rs", "FOX", "the imported game's movie file stem"),
    ("frontend/movie.rs", "GREG", "the imported game's movie file stem"),
    ("fight/br1/frame_tests.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("fight/sim/br1_tests.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("fight/sim/ko_replay.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("fight/team/br1_tests.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    ("session/replay/tests.rs", "FOX", "the imported game's fighter code (its files' stem)"),
    // the neutral placeholder of a stage's name without the game's text
    ("assets/text.rs", "STAGE {}", "the placeholder of a stage name (no game data)"),
    // the default rankings' name: the save's default state, built before any game data may be loaded
    ("frontend/options.rs", "HUD", "the default rankings' name in the save's default state"),
    // the setup screen: shown before any game data exists
    ("app/setup/screen.rs", "BACK", "the setup screen, before any game data exists"),
    ("app/setup/screen.rs", "QUIT", "the setup screen, before any game data exists"),
    // the tools' environment switches and their own match of the mode menu
    ("bin/sim.rs", "STAGE", "an environment variable's name"),
    ("bin/sim.rs", "PRACTICE", "an environment variable's name"),
    ("bin/sim.rs", "ARCADE", "an environment variable's name"),
    ("bin/fetest.rs", "WINS", "an environment variable's name"),
    ("bin/br1test.rs", "STAGE", "an environment variable's name"),
    ("bin/nettest.rs", "WATCH", "the netplay harness reads the mode menu's row"),
    ("bin/nettest.rs", "VS", "the netplay harness reads the mode menu's row"),
];

/// The texts and keys of a converted text table (`key NUL text NUL` …), None without the file.
fn table(root: &str) -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let b = std::fs::read(Path::new(root).join("engine/tables.okt")).ok()?;
    let t = open_kumite_beast::formats::tables::Tables::parse(&b).expect("tables.okt");
    let blob = t.get(open_kumite_beast::assets::text::TABLE).expect("a text table").bytes();
    let pairs = open_kumite_beast::assets::text::parse(blob).expect("text table");
    Some((pairs.iter().map(|p| p.0.to_string()).collect(), pairs.iter().map(|p| p.1.to_string()).collect()))
}

/// `s` with a format string's arguments (`{…}`) and a text's value marks (`%s`, `%u`, `%d`) as one mark, `{{` / `}}` as
/// braces, the spaces at its ends cut and each run of spaces inside as one.
fn normal(s: &str) -> String {
    let s = s.replace("{{", "\u{2}").replace("}}", "\u{3}").replace("%s", "\u{1}").replace("%u", "\u{1}").replace("%d", "\u{1}");
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '{' => {
                depth += 1;
                if depth == 1 {
                    out.push('\u{1}');
                }
            }
            '}' if depth > 0 => depth -= 1,
            _ if depth > 0 => {}
            '\u{2}' => out.push('{'),
            '\u{3}' => out.push('}'),
            c => out.push(c),
        }
    }
    out.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `s` (`normal`) with each run of digits as the value mark.
fn numbers_marked(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if !c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('\u{1}') {
            out.push('\u{1}');
        }
    }
    out
}

/// A literal's text: its escapes `\"`, `\\`, `\n`, `\t`, `\'` read (the rest kept as written).
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(c @ ('"' | '\\' | '\'')) => out.push(c),
            Some(c) => {
                out.push('\\');
                out.push(c);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The string literals of `src` outside comments: (byte offset, contents as written).
fn literals(src: &str) -> Vec<(usize, String)> {
    let s = structure(src);
    let quotes: Vec<usize> = s.match_indices('"').map(|(k, _)| k).collect();
    quotes.chunks(2).filter(|q| q.len() == 2).map(|q| (q[0], src[q[0] + 1..q[1]].to_string())).collect()
}

/// The keys a call names in `src`: (byte offset, game 1 or 2, key).
fn keys(src: &str) -> Vec<(usize, u8, String)> {
    let mut out = Vec::new();
    for (call, game) in [("text::get(\"", 2), ("text::item(\"", 2), ("Label::Game(\"", 2), ("text::br1(\"", 1), ("text::br1_item(\"", 1)] {
        for (k, _) in src.match_indices(call) {
            let start = k + call.len();
            let Some(len) = src[start..].find('"') else { continue };
            let key = &src[start..start + len];
            let key = if call.contains("item") { format!("{key}.0") } else { key.to_string() };
            out.push((k, game, key));
        }
    }
    out
}

#[test]
fn no_original_text_in_code() {
    let br2_root = open_kumite_beast::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
    let br1_root = std::env::var("BR1_EXPORT").unwrap_or_else(|_| format!("{br2_root}/br1"));
    let Some((br2_keys, br2_texts)) = table(&br2_root) else {
        eprintln!("skipped: no converted main-game data");
        return;
    };
    let (br1_keys, br1_texts) = table(&br1_root).unwrap_or_default();
    let texts: BTreeSet<String> = br2_texts.iter().chain(&br1_texts).map(|t| normal(t)).filter(|t| t.chars().count() >= 2).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests"] {
        let mut rels = Vec::new();
        collect(&root.join(dir), &root.join(dir), &mut rels);
        files.extend(rels.into_iter().map(|r| (dir, r)));
    }
    let mut failures = Vec::new();
    let mut used = BTreeSet::new();
    for (dir, rel) in &files {
        let src = std::fs::read_to_string(root.join(dir).join(rel)).expect("read");
        let rel = if *dir == "src" { rel.clone() } else { format!("{dir}/{rel}") };
        let rel = rel.as_str();
        let starts: Vec<usize> = std::iter::once(0).chain(src.match_indices('\n').map(|(k, _)| k + 1)).collect();
        // this file's own allow-list names the texts it lets pass
        let from = if rel == "tests/original_text.rs" { src.find("const ALLOWED").and_then(|a| src[a..].find("\n];").map(|e| a + e)).unwrap_or(0) } else { 0 };
        for (at, lit) in literals(&src).into_iter().filter(|l| l.0 >= from) {
            // as written, or with its numbers as the value mark (a text filled in: "… %u" as "… 1")
            let lit = normal(&unescape(&lit));
            let Some(lit) = [lit.clone(), numbers_marked(&lit)].into_iter().find(|l| texts.contains(l)) else { continue };
            match ALLOWED.iter().find(|a| a.0 == rel && normal(a.1) == lit) {
                Some(a) => {
                    used.insert((a.0, a.1));
                }
                None => failures.push(format!("{rel}:{}: \"{lit}\" is the game's text: take it from the text table", line_of(&starts, at))),
            }
        }
        let code = strip_comments(&src);
        for (at, game, key) in keys(&code) {
            let known = if game == 2 { &br2_keys } else { &br1_keys };
            if !(known.is_empty() || known.contains(&key)) {
                failures.push(format!("{rel}:{}: no text {key} in the game's table", line_of(&starts, at)));
            }
        }
    }
    for a in ALLOWED.iter().filter(|a| !used.contains(&(a.0, a.1))) {
        failures.push(format!("ALLOWED {} \"{}\" matches nothing any more: remove it", a.0, a.1));
    }
    assert!(failures.is_empty(), "{} finding(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn literal_scanning() {
    let src =
        "let a = \"ABC DEF\"; // \"IN A COMMENT\"\nlet b = format!(\"ABC? {}\", n); let c = '\"';\n#[cfg(test)]\nmod tests { const X: &str = \"TEST\"; }\n";
    let l: Vec<String> = literals(src).into_iter().map(|l| l.1).collect();
    assert_eq!(l, ["ABC DEF", "ABC? {}", "TEST"]);
    assert_eq!(normal("ABC? {}"), normal("ABC? %u"));
    assert_eq!(normal("  ABC  DEF {}  "), normal("ABC DEF %u"));
    assert_eq!(normal("{{a}} {x:2} "), "{a} \u{1}");
    assert_eq!(numbers_marked("ABC 12"), normal("ABC %u"));
    assert_eq!(unescape(r#"A\"B\\n"#), "A\"B\\n");
    let k = keys("text::get(\"you_win\") text::item(\"stages\", 3) Label::Game(\"exit\") text::br1(\"final_stage\")");
    let k: Vec<(u8, &str)> = k.iter().map(|(_, g, s)| (*g, s.as_str())).collect();
    assert_eq!(k, [(2, "you_win"), (2, "stages.0"), (2, "exit"), (1, "final_stage")]);
}
