//! Reference check (engine addition): the source, the docs and the build files name no address of the program the
//! engine recreates, no reverse-engineering note and no porting tag. It scans code and comments alike in `src/**`,
//! `tests/**`, the top-level `*.md`, `docs/**`, `Cargo.toml`, `build.rs`, `index.html`, `deny.toml`, `rustfmt.toml`,
//! the git and hook files, `android/` (its Java, resources, manifest and build script) and `fuzz/` (its targets and
//! manifest), and fails on a line that matches one of `PATTERNS` unless `ALLOWED` lists the file and the pattern
//! (`ALLOWED_CODE`: on a code line only, a comment line still fails). An entry of either list that matches nothing
//! fails too (the lists only shrink).
//!
//! `game_names` holds comments and docs to neutral names (the main game, the imported game): the games' names may
//! stand only in string literals (a `.rs` file) or code spans (a `.md` doc) of the files `NAMES_IN_LITERALS` lists,
//! never on a `log::` line. A name wrapped across a comment's line break counts too. Identifiers are out of scope
//! (the converted data's ids keep their `br1` / `br2` names).

mod common;

use std::path::{Path, PathBuf};

/// Files that may match a pattern: (path from the repository root, the pattern's name, why).
const ALLOWED: &[(&str, &str, &str)] = &[
    // the reference runners and their tests: they read recorded runs of the game and compare the engine with them,
    // so they name the game's addresses, its block places and the recordings' tools
    ("src/bin/br1test.rs", "address", "a reference runner: the recorded runs' addresses"),
    ("src/bin/br1test.rs", "scratchpad", "a reference runner: the recorded runs' fighter blocks"),
    ("src/bin/br1test.rs", ".research/", "a reference runner: where the recording scripts live"),
    ("src/bin/br1test.rs", "port addition", "a reference runner's header"),
    ("src/bin/br1test.rs", "PCSX", "a reference runner: the recordings' emulator"),
    ("src/bin/cliptest.rs", "address", "a reference runner: the recorded dumps' addresses"),
    ("src/bin/cliptest.rs", "scratchpad", "a reference runner: the recorded dumps' fighter blocks"),
    ("src/bin/cliptest.rs", "PCSX", "a reference runner: the recorded dumps' emulator"),
    ("src/bin/cliptest.rs", ".research/", "a reference runner: where the recorded dumps live"),
    ("src/bin/cliptest.rs", "port addition", "a reference runner's header"),
    ("src/fight/br1/reference.rs", "address", "a reference runner's core: the call log's return addresses"),
    ("src/fight/br1/reference.rs", "scratchpad", "a reference runner's core: the recorded fighter blocks"),
    ("src/fight/br1/reference.rs", "Original", "a reference runner's core: what the recordings hold"),
    ("src/fight/br1/reference.rs", "Spec:", "a reference runner's core"),
    ("src/fight/br1/reference.rs", "docs/specs", "a reference runner's core"),
    ("src/fight/br1/reference.rs", "PCSX", "a reference runner's core: the recordings' emulator"),
    ("src/fight/br1/reference.rs", ".research/", "a reference runner's core: where the recording scripts live"),
    ("src/fight/br1/reference.rs", "port addition", "a reference runner's header"),
    ("src/fight/br1/reference.rs", "spec section", "a reference runner's core"),
    ("src/fight/br1/reference_frame.rs", "address", "a reference runner's core: the call log's return addresses"),
    ("src/fight/br1/reference_frame.rs", "Original", "a reference runner's core: what the recordings hold"),
    ("src/fight/br1/reference_frame.rs", "Spec:", "a reference runner's core"),
    ("src/fight/br1/reference_frame.rs", "docs/specs", "a reference runner's core"),
    ("src/fight/br1/reference_frame.rs", "PCSX", "a reference runner's core: the recordings' emulator"),
    ("src/fight/br1/reference_frame.rs", ".research/", "a reference runner's core: where the recording scripts live"),
    ("src/fight/br1/reference_frame.rs", "port addition", "a reference runner's header"),
    ("src/fight/br1/reference_frame.rs", "spec section", "a reference runner's core"),
    ("src/fight/br1/script/tests.rs", "address", "the reference cases: the recorded runs' addresses"),
    ("src/fight/br1/script/tests.rs", ".research/", "the reference cases: where the recorded runs live"),
    ("src/fight/br1/script/tests.rs", "spec section", "the reference cases: the script census"),
    ("src/fight/br1/fixtures/move_sheets.txt", ".research/", "a fixture recorded from the game: its header"),
    ("src/present/fixtures/br1_drop.txt", "address", "a fixture recorded from the game: its header"),
    ("src/present/fixtures/br1_drop.txt", ".research/", "a fixture recorded from the game: its header"),
    // The imported game's clip tables hold pointers into its program image
    ("src/fight/br1/address_map.rs", "address", "the imported game's data: its clip pointers into the program image"),
    // the setup tells the two discs apart by their boot file, and removes an older install's program file
    ("src/app/setup/plan.rs", "SCUS", "the disc check: the boot file names which disc was chosen"),
    ("src/app/setup/convert.rs", "SCUS", "a test: an older install's program file is removed by a new conversion"),
    // the golden baseline and the determinism scripts live beside the repository
    ("CONTRIBUTING.md", ".research/", "the baseline and determinism scripts outside the repository"),
    ("docs/determinism.md", ".research/", "the determinism scripts outside the repository"),
];

/// Files whose code lines may match a pattern (their comments may not): (path, the pattern's name, why).
const ALLOWED_CODE: &[(&str, &str, &str)] = &[
    // the converted data's own address spaces: the effect scripts address the match block, the models, the globals
    // and the ring by these addresses, and hold the callback words the effect objects carry
    ("src/fight/fx/script_link.rs", "address", "the effect scripts' address map"),
    ("src/fight/fx/script_link.rs", "scratchpad", "the effect scripts' address map"),
];

/// The games' names, as their two words (case-sensitive: the setup's own labels, ADD BLOODY ROAR 1 DISC, name the
/// disc to insert).
const GAME_NAME: (&str, &str) = ("Bloody", "Roar");

/// Files whose string literals (`.rs`) or code spans (docs) may name the games, never their comments or prose:
/// (path from the repository root, why).
const NAMES_IN_LITERALS: &[(&str, &str)] = &[
    // what the player reads to pick the right disc (the user's decision: the disc keeps its name there)
    ("src/app/setup/draw.rs", "the setup's disc prompts"),
    ("src/app/setup/pickers.rs", "the file pickers' titles: which disc image or data to choose"),
    ("src/app/setup/plan.rs", "the setup's disc messages: which disc was chosen, which is wanted"),
    ("src/app/setup/job.rs", "the setup's message naming the second disc's data a folder lacks"),
    ("src/app/setup/web.rs", "the browser setup's message naming the second disc's data a folder lacks"),
    ("src/app/setup/convert.rs", "the setup's free-space message naming the disc data; the disc tests' image paths"),
    ("src/assets/manifest.rs", "the setup's state lines of the second disc; the converted data's disc id (`BR1_GAME`)"),
    ("src/net/handshake.rs", "netplay's refusal naming the disc data the other side lacks"),
    ("src/session/replay/mod.rs", "a replay's refusal naming the disc data it was recorded with"),
    ("src/platform/game_data.rs", "an install error naming the disc data that could not be moved"),
    // test disc paths: the developer's own disc images
    ("src/formats/xa.rs", "the XA test's disc image path"),
    ("CONTRIBUTING.md", "the disc tests' image paths"),
    ("docs/debug_hooks.md", "the disc tests' default image paths"),
    // tests of the above: a manifest holding the second disc's id, and checks that a refusal names the disc
    ("src/assets/extras.rs", "a test manifest holding the second disc's id"),
    ("src/session/replay/tests.rs", "checks that a replay's refusal names the disc data it lacks"),
];

/// The patterns: (name, matcher).
const PATTERNS: &[(&str, fn(&str) -> bool)] = &[
    ("address", address),
    ("scratchpad", scratchpad),
    ("Original", |l| word_then(l, "Original", &[':', '('])),
    ("Spec:", |l| word_then(l, "Spec", &[':'])),
    ("docs/specs", |l| l.contains("docs/specs")),
    ("SCUS", |l| l.contains("SCUS_94")),
    ("exe.asm", |l| l.contains("exe.asm")),
    ("PCSX", |l| l.contains("PCSX")),
    ("disassembl", |l| l.contains("disassembl")),
    (".research/", |l| l.contains(".research/")),
    ("spec section", spec_section),
    ("port addition", |l| l.contains("port addition") || l.contains("Port addition")),
    ("tr_*.c", other_source_file),
];

/// A source file of another engine's renderer, `tr_<name>.c` (`tr_sky.c`): the engine describes behaviour, never
/// the C code it may resemble.
fn other_source_file(l: &str) -> bool {
    let b = l.as_bytes();
    l.match_indices("tr_").any(|(i, _)| {
        (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) && {
            let rest = &l[i + 3..];
            let n = rest.bytes().take_while(|c| c.is_ascii_lowercase()).count();
            n > 0 && rest[n..].starts_with(".c") && !rest[n + 2..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        }
    })
}

fn hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// `0x` / `0X` + `prefix` + four hex digits with an optional `_` before them (`0x801B_14DC`, `0x801b14dc`), not part
/// of a longer number (a type suffix may follow: `0x8017_9C38_u32`).
fn hex_word(l: &str, prefix: &[fn(u8) -> bool]) -> bool {
    let b = l.as_bytes();
    let lower = l.to_ascii_lowercase();
    let mut i = 0;
    while let Some(p) = lower[i..].find("0x").map(|p| i + p) {
        i = p + 2;
        if p > 0 && (b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_') {
            continue;
        }
        let mut j = p + 2;
        if !prefix.iter().all(|f| {
            b.get(j).is_some_and(|&c| f(c)) && {
                j += 1;
                true
            }
        }) {
            continue;
        }
        if b.get(j) == Some(&b'_') {
            j += 1;
        }
        let more = |k: usize| b.get(k).is_some_and(|&c| hex(c));
        if (0..4).all(|k| more(j + k)) && !more(j + 4) && !(b.get(j + 4) == Some(&b'_') && more(j + 5)) {
            return true;
        }
    }
    false
}

/// An address in the program's code and data, 0x8010_0000..0x801F_FFFF (smaller values starting 0x800 are flag words
/// such as the sign bit, 0x8000_0000).
fn address(l: &str) -> bool {
    hex_word(l, &[|c| c == b'8', |c| c == b'0', |c| c == b'1', hex])
}

/// An address in the console's scratchpad, 0x1F80_0000..0x1F80_FFFF.
fn scratchpad(l: &str) -> bool {
    hex_word(l, &[|c| c == b'1', |c| c == b'F' || c == b'f', |c| c == b'8', |c| c == b'0'])
}

/// `word` as a whole word followed by one of `next`.
fn word_then(l: &str, word: &str, next: &[char]) -> bool {
    let b = l.as_bytes();
    l.match_indices(word).any(|(i, _)| (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) && l[i + word.len()..].starts_with(next))
}

/// A section of a research note: `name.md §`.
fn spec_section(l: &str) -> bool {
    l.match_indices(".md §").any(|(i, _)| l[..i].chars().next_back().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
}

/// The files to scan, as paths from the repository root.
fn files(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    for d in ["src", "tests", "docs", "fuzz/fuzz_targets", "android/java", "android/res", ".cargo-husky", "tools"] {
        walk(&root.join(d), &mut v);
    }
    for f in [
        "Cargo.toml",
        "build.rs",
        "index.html",
        "deny.toml",
        "rustfmt.toml",
        ".gitignore",
        ".gitattributes",
        "fuzz/Cargo.toml",
        "fuzz/.gitignore",
        "android/AndroidManifest.xml",
        "android/build.sh",
    ] {
        v.push(root.join(f));
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        v.extend(rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")));
    }
    let mut v: Vec<String> = v
        .into_iter()
        .filter(|p| p.is_file())
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
        .filter(|p| p != "tests/no_original_refs.rs")
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn no_original_refs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (mut found, mut used, mut used_code) = (Vec::new(), vec![false; ALLOWED.len()], vec![false; ALLOWED_CODE.len()]);
    let mut scanned = 0;
    for f in files(root) {
        // binary fixtures (replays, saves, images) are not text
        let Ok(text) = std::fs::read_to_string(root.join(&f)) else { continue };
        scanned += 1;
        for (n, line) in text.lines().enumerate() {
            for (name, hit) in PATTERNS {
                if !hit(line) {
                    continue;
                }
                let allowed = |list: &[(&str, &str, &str)]| list.iter().position(|(af, ap, _)| *af == f && ap == name);
                if let Some(k) = allowed(ALLOWED) {
                    used[k] = true;
                } else if let Some(k) = allowed(ALLOWED_CODE).filter(|_| code_only(line, *hit)) {
                    used_code[k] = true;
                } else {
                    found.push(format!("{f}:{}: {name}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    for (list, entries, used) in [("ALLOWED", ALLOWED, &used), ("ALLOWED_CODE", ALLOWED_CODE, &used_code)] {
        for (k, (f, p, why)) in entries.iter().enumerate() {
            if !used[k] {
                found.push(format!("{list} ({f}, {p}, {why}) matches nothing: remove it"));
            }
        }
    }
    found.extend(game_names(root));
    assert!(scanned > 300, "only {scanned} files scanned");
    assert!(found.is_empty(), "references to the original program: {} finding(s)\n{}", found.len(), found.join("\n"));
}

/// The games' names in comments and prose, on a `log::` line, or in a literal of a file `NAMES_IN_LITERALS` does not
/// list; and a listed file that names them nowhere.
fn game_names(root: &Path) -> Vec<String> {
    let (mut found, mut used) = (Vec::new(), vec![false; NAMES_IN_LITERALS.len()]);
    for f in files(root) {
        let Ok(text) = std::fs::read_to_string(root.join(&f)) else { continue };
        let hits = name_hits(&text);
        if hits.is_empty() {
            continue;
        }
        // a literal span only in Rust (its strings) and Markdown (its code spans); elsewhere all of it is prose
        let code = if f.ends_with(".rs") {
            rs_code(&text)
        } else if f.ends_with(".md") {
            doc_code(&text)
        } else {
            vec![false; text.len()]
        };
        for p in hits {
            let line = text[..p].matches('\n').count() + 1;
            let line_text = text.lines().nth(line - 1).unwrap_or("");
            if !code[p] {
                found.push(format!("{f}:{line}: a game's name in a comment or prose"));
            } else if line_text.contains("log::") {
                found.push(format!("{f}:{line}: a game's name in a log line"));
            } else if let Some(k) = NAMES_IN_LITERALS.iter().position(|(af, _)| *af == f) {
                used[k] = true;
            } else {
                found.push(format!("{f}:{line}: a game's name in a literal (not a listed disc prompt)"));
            }
        }
    }
    for (k, (f, why)) in NAMES_IN_LITERALS.iter().enumerate() {
        if !used[k] {
            found.push(format!("NAMES_IN_LITERALS ({f}, {why}) matches nothing: remove it"));
        }
    }
    found
}

/// Where `text` names a game: the first word's byte offsets, the two words apart by a space or wrapped across a
/// comment's line break (whitespace and the comment markers `/`, `!`, `*` between them).
fn name_hits(text: &str) -> Vec<usize> {
    let (first, second) = GAME_NAME;
    text.match_indices(first)
        .filter(|&(p, _)| {
            let rest = &text[p + first.len()..];
            let gap = rest.bytes().take_while(|c| c.is_ascii_whitespace() || matches!(c, b'/' | b'!' | b'*')).count();
            rest.as_bytes().first().is_some_and(u8::is_ascii_whitespace) && rest[gap..].starts_with(second)
        })
        .map(|(p, _)| p)
        .collect()
}

/// Per byte of a Rust file: whether it is code or a literal (not a comment). `strip_comments` blanks every comment
/// byte to a space and keeps the byte length, so a byte that is not a space there is code.
fn rs_code(text: &str) -> Vec<bool> {
    common::strip_comments(text).bytes().map(|b| b != b' ').collect()
}

/// Per byte of a doc: whether it is inside a code span or a fenced block. A code span never crosses a paragraph, so a
/// blank line ends an unclosed one (a stray backtick hides at most the rest of its paragraph).
fn doc_code(text: &str) -> Vec<bool> {
    let mut out = vec![false; text.len()];
    let (mut fence, mut span, mut at) = (false, false, 0);
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            out[at..at + line.len()].fill(true);
        } else if fence {
            out[at..at + line.len()].fill(true);
        } else if line.trim().is_empty() {
            span = false;
        } else {
            for (i, c) in line.bytes().enumerate() {
                if c == b'`' {
                    span = !span;
                }
                out[at + i] = span || c == b'`';
            }
        }
        at += line.len();
    }
    out
}

/// An `ALLOWED_CODE` hit counts only in the code: the line is not a comment line (`//`, `/*`, `*`), the part before
/// a trailing `//` matches and the comment after it does not.
fn code_only(line: &str, hit: fn(&str) -> bool) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
        return false;
    }
    let (code, comment) = line.split_once("//").unwrap_or((line, ""));
    hit(code) && !hit(comment)
}

#[test]
fn patterns() {
    assert!(code_only("const RING: u32 = 0x801F_06E0;", address));
    assert!(!code_only("const RING: u32 = 0x801F_06E0; // also read by 0x801B_0B60", address));
    assert!(!code_only("const RING: u32 = 1; // 0x801B_0B60", address));
    assert!(!code_only("   * 0x801B_0B60 in a block comment", address));
    assert!(address("x = 0x801B_14DC;") && address("0x8017e824") && address("(0x80140880)"));
    assert!(!address("0x8000_0000") && !address("0x8008_0000") && !address("0x80_0000") && !address("0x80140880AB"));
    assert!(address("0x8017_9C38_u32") && address("0x80179c38_u32") && address("0X80179C38") && address("0X8017_9c38"));
    assert!(!address("0x8017_9C38_0000") && !address("0x80179C38_0"));
    assert!(scratchpad("0x1F80_0000") && scratchpad("0x1f8000e8") && !scratchpad("0x1F8"));
    assert!(word_then("//! Original: 0x", "Original", &[':', '(']));
    assert!(!word_then("the Originals:", "Original", &[':', '(']) && !word_then("Original text", "Original", &[':', '(']));
    assert!(spec_section("(team_battle.md §4)") && !spec_section("see docs/android.md, §"));
    assert!(other_source_file("(`tr_sky.c` face layout)") && other_source_file("tr_shade.c:") && !other_source_file("attr_sky.c"));
    assert!(!other_source_file("tr_sky.cpp") && !other_source_file("tr_.c") && !other_source_file("tr_sky.cfg"));
    assert_eq!(name_hits("// a Bloody\n// Roar 1 fighter"), [5]);
    assert_eq!(name_hits("    /// a Bloody\n    /// Roar II fighter, //! Bloody\n//!   Roar"), [10, 46]);
    assert!(name_hits("Bloody Roar").len() == 1 && name_hits("Bloody\n * Roar").len() == 1);
    assert!(name_hits("Bloody% Roar").is_empty() && name_hits("BloodyRoar").is_empty() && name_hits("Bloody rare").is_empty());
    let rs = "let s = \"Bloody Roar\"; // Bloody Roar";
    let c = rs_code(rs);
    assert!(c[rs.find('B').unwrap()] && !c[rs.rfind('B').unwrap()]);
    let md = "`a/Bloody Roar\n(USA).cue` and Bloody Roar";
    let c = doc_code(md);
    assert!(c[md.find('B').unwrap()] && !c[md.rfind('B').unwrap()]);
    let md = "a stray ` here\n\nBloody Roar";
    assert!(!doc_code(md)[md.find('B').unwrap()]);
}
