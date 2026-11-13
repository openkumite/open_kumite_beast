//! Version-gate check (engine addition, refactor R2): a simulation version is compared with a rule's first version in
//! `src/fight/version.rs` only (`Gates`, the constants of `v`); everywhere else the logic asks a rule's flag. It scans
//! `src/**/*.rs` outside test code (a `tests.rs` / `*_tests.rs` file, a `#[cfg(test)]` inline module) and comments,
//! and fails on `sim_version` beside a comparison operator (`sim_version >= N`, `N < data.sim_version`, …) that
//! `ALLOWED` does not list. An entry of `ALLOWED` that matches nothing fails too (the list only shrinks).

use std::path::Path;

/// Comparisons that are not a rule's gate: (file under `src/`, the line's text, why).
const ALLOWED: &[(&str, &str, &str)] = &[
    ("session/replay/mod.rs", "if sim_version > SIM_VERSION {", "a replay file's version newer than this build"),
    ("session/replay/mod.rs", "if sim_version == 0 {", "a replay file's version 0"),
];

const OPS: [&str; 6] = ["<=", ">=", "==", "!=", "<", ">"];

/// `line` without its `//` comment and the contents of its string literals (a naive cut: enough for this source).
fn code_of(line: &str) -> String {
    let mut out = String::new();
    let (mut in_str, mut prev) = (false, ' ');
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            if c == '"' && prev != '\\' {
                in_str = false;
                out.push('"');
            }
            prev = if prev == '\\' && c == '\\' { ' ' } else { c };
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            break;
        }
        if c == '"' {
            in_str = true;
            prev = ' ';
        }
        out.push(c);
    }
    out
}

/// Whether `code` compares `sim_version` (a word, maybe a field or a method: `data.sim_version`, `d.sim_version()`)
/// with an operator, matches it on patterns (`match sim_version {`, `matches!(sim_version, …)`,
/// `(a..b).contains(&sim_version)`) or calls `Gates::reaches` with a number (its argument is a `v::` constant or a
/// table's version field).
fn compares(code: &str) -> bool {
    compares_word(code, "sim_version") || reaches_number(code)
}

/// `compares` on `word` (`sim_version`, or a local alias of it: `aliases`).
fn compares_word(code: &str, word: &str) -> bool {
    let b = code.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    while let Some(i) = code[from..].find(word).map(|i| from + i) {
        let mut end = i + word.len();
        from = end;
        if (i > 0 && is_word(b[i - 1])) || b.get(end).is_some_and(|&c| is_word(c)) {
            continue;
        }
        if code[end..].starts_with("()") {
            end += 2;
        }
        // after it: an operator (not `->` / `=>` / `>>` / `<<`)
        let after = code[end..].trim_start();
        if OPS.iter().any(|op| after.starts_with(op)) && !after.starts_with("<<") && !after.starts_with(">>") {
            return true;
        }
        // before it (past a field path `a.b.`): an operator that is not `=>` / `->`
        let head = code[..i].trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.').trim_end();
        if let Some(op) = OPS.iter().find(|op| head.ends_with(*op)) {
            let rest = &head[..head.len() - op.len()];
            let arrow = *op == ">" && (rest.ends_with('=') || rest.ends_with('-'));
            if !arrow && !(*op == "<" && rest.ends_with('<')) && !(*op == ">" && rest.ends_with('>')) {
                return true;
            }
        }
        // a pattern match on it: `match v {`, `matches!(v, …)`, `(a..b).contains(&v)`
        if (head.ends_with("match") && after.starts_with('{'))
            || (head.ends_with("matches!(") && after.starts_with(','))
            || (head.ends_with(".contains(&") && after.starts_with(')'))
        {
            return true;
        }
    }
    false
}

/// Whether `code` calls `reaches` with a number (`g.reaches(22)`) rather than a `v::` constant.
fn reaches_number(code: &str) -> bool {
    code.match_indices("reaches(").any(|(i, m)| code[i + m.len()..].trim_start().starts_with(|c: char| c.is_ascii_digit()))
}

/// The local aliases of the version in `lines`: `let v = data.sim_version;` (a plain read, maybe `()`, `as T` or
/// `.unwrap_or(…)`), with the range of `lines` they live in (to the end of the enclosing block), so that `v >= 9` is
/// caught as `sim_version >= 9` is.
fn aliases(lines: &[(usize, &str)]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    for (k, (_, line)) in lines.iter().enumerate() {
        let code = code_of(line);
        let Some(rest) = code.trim().strip_prefix("let ") else { continue };
        let Some((name, rhs)) = rest.split_once('=') else { continue };
        let name = name.split(':').next().unwrap_or("").trim().trim_start_matches("mut ").trim();
        if name == "sim_version" || name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            continue;
        }
        let rhs = rhs.trim().trim_end_matches(';').trim();
        let rhs = rhs.split(" as ").next().unwrap_or(rhs);
        let rhs = rhs.split(".unwrap_or").next().unwrap_or(rhs).trim_end_matches("()");
        let plain = rhs.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.');
        if !(plain && (rhs == "sim_version" || rhs.ends_with(".sim_version"))) {
            continue;
        }
        // its block: until the brace depth falls below the `let`'s
        let mut depth = 0i32;
        let mut end = lines.len();
        for (j, (_, l)) in lines.iter().enumerate().skip(k + 1) {
            let c = code_of(l);
            depth += c.matches('{').count() as i32 - c.matches('}').count() as i32;
            if depth < 0 {
                end = j;
                break;
            }
        }
        out.push((name.to_string(), k + 1..end));
    }
    out
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).expect("src") {
        let p = e.expect("entry").path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The lines of `text` outside `#[cfg(test)]` inline modules (by brace depth, on the code without strings and
/// comments), with their numbers.
fn non_test_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < lines.len() {
        let t = lines[k].trim();
        if t.starts_with("#[cfg(") && t.contains("test") {
            // the item it gates: an inline `mod x {` is skipped to its closing brace; `mod x;` is a file (skipped by name)
            let next = (k + 1..lines.len()).find(|&j| !lines[j].trim().is_empty() && !lines[j].trim().starts_with("#["));
            if let Some(j) = next.filter(|&j| lines[j].trim_start().starts_with("mod ") && lines[j].contains('{')) {
                let mut depth = 0i32;
                let mut m = j;
                while m < lines.len() {
                    let c = code_of(lines[m]);
                    depth += c.matches('{').count() as i32 - c.matches('}').count() as i32;
                    m += 1;
                    if depth <= 0 {
                        break;
                    }
                }
                k = m;
                continue;
            }
        }
        out.push((k + 1, lines[k]));
        k += 1;
    }
    out
}

#[test]
fn version_comparisons_are_in_version_rs_only() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    let mut found = Vec::new();
    let mut used = vec![false; ALLOWED.len()];
    for f in &files {
        let rel = f.strip_prefix(&src).expect("under src").to_string_lossy().replace('\\', "/");
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if rel == "fight/version.rs" || stem == "tests" || stem.ends_with("_tests") {
            continue;
        }
        let text = std::fs::read_to_string(f).expect("source");
        let lines = non_test_lines(&text);
        let aliases = aliases(&lines);
        for (k, &(n, line)) in lines.iter().enumerate() {
            let code = code_of(line);
            let alias = aliases.iter().any(|(a, live)| live.contains(&k) && compares_word(&code, a));
            if !compares(&code) && !alias {
                continue;
            }
            match ALLOWED.iter().position(|(file, l, _)| *file == rel && line.trim() == *l) {
                Some(a) => used[a] = true,
                None => found.push(format!("src/{rel}:{n}: {}", line.trim())),
            }
        }
    }
    let stale: Vec<_> = ALLOWED.iter().zip(&used).filter(|(_, u)| !**u).map(|(a, _)| format!("{a:?}")).collect();
    assert!(
        found.is_empty() && stale.is_empty(),
        "a raw sim_version comparison outside src/fight/version.rs (ask a rule of fight::version::Gates instead):\n{}\nALLOWED entries that match nothing (remove them):\n{}",
        found.join("\n"),
        stale.join("\n")
    );
}

/// The scanner itself.
#[test]
fn compares_finds_both_sides_and_not_arrows() {
    assert!(compares("if data.sim_version >= 7 {"));
    assert!(compares("x && 7 <= s.sim_version"));
    assert!(compares("a.sim_version == b"));
    assert!(!compares("Some(t) => sim_version,"));
    assert!(!compares("fn f() -> sim_version"));
    assert!(!compares("let sim_version_now = 3 >= x;"));
    assert!(!compares("Gates::of(data.sim_version).team"));
    assert!(!compares(&code_of("x // sim_version >= 3")));
    assert!(!compares(&code_of("format!(\"sim_version >= {}\", 3)")));
    assert_eq!(non_test_lines("a\n#[cfg(test)]\nmod tests {\n    b {\n    }\n}\nc").iter().map(|l| l.1).collect::<Vec<_>>(), ["a", "c"]);
}

/// The scanner's aliases, pattern matches, method reads and `reaches` with a number.
#[test]
fn compares_finds_aliases_patterns_and_reaches() {
    assert!(compares("if data.sim_version() >= 7 {"));
    assert!(compares("match data.sim_version {"));
    assert!(compares("let new = matches!(d.sim_version(), 3..=9);"));
    assert!(compares("if (3..9).contains(&sim_version) {"));
    assert!(compares("g.reaches(22)"));
    assert!(!compares("g.reaches(v::TEAM)"));
    assert!(!compares("match load(&a, sim_version).await {"));
    let text = "fn f(data: &D) {\n    let v = data.sim_version;\n    if v >= 9 {\n    }\n}\nfn g() {\n    if v >= 255 {\n    }\n}";
    let lines: Vec<(usize, &str)> = text.lines().enumerate().map(|(k, l)| (k + 1, l)).collect();
    let a = aliases(&lines);
    assert_eq!(a, [("v".to_string(), 2..4)]);
    assert!(a[0].1.contains(&2) && compares_word(lines[2].1, "v") && !a[0].1.contains(&6));
}
