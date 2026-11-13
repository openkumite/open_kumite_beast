//! Rust source scanning shared by the source checks (`layering.rs`, `original_text.rs`): comments and literals
//! blanked by byte offset, the inline modules, test code.

#![allow(dead_code)]

use std::path::Path;

/// `src` with comments blanked (newlines kept, so byte offsets keep their line); string and char literals stay.
pub fn strip_comments(src: &str) -> String {
    lex(src, false)
}

/// `src` with comments and the insides of string and char literals blanked (the quotes stay): its braces are the
/// code's own, for the inline modules (`inline_mods`).
pub fn structure(src: &str) -> String {
    lex(src, true)
}

/// Blanks comments, and with `literals` also what is inside the string and char literals. Every blanked byte becomes
/// a space (newlines kept), so byte offsets and lines stay.
pub fn lex(src: &str, literals: bool) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to.min(b.len())] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| i + p);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let (mut depth, mut j) = (1, i + 2);
                while j < b.len() && depth > 0 {
                    if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                        depth += 1;
                        j += 2;
                    } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j);
                i = j;
            }
            // raw strings r"…", r#"…"#, and the raw byte / C strings br"…", cr#"…"#
            b'r' if (!ident_before(b, i) || (i > 0 && matches!(b[i - 1], b'b' | b'c') && !ident_before(b, i - 1)))
                && matches!(b.get(i + 1), Some(b'"' | b'#')) =>
            {
                let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
                if b.get(i + 1 + hashes) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
                let start = i + 2 + hashes;
                let end = b[start..].windows(close.len()).position(|w| w == close.as_slice()).map_or(b.len(), |p| start + p);
                if literals {
                    blank(&mut out, start, end);
                }
                i = (end + close.len()).min(b.len());
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                if literals {
                    blank(&mut out, i + 1, j);
                }
                i = j + 1;
            }
            // a char literal ('x', '\n', '\u{..}'); otherwise a lifetime
            b'\'' => {
                let end = if b.get(i + 1) == Some(&b'\\') {
                    // '\x' then up to the closing quote ('\'', '\n', '\u{…}')
                    b.get(i + 3..).and_then(|r| r.iter().position(|&c| c == b'\'')).map_or(b.len(), |p| i + 4 + p)
                } else if b.get(i + 2) == Some(&b'\'') {
                    i + 3
                } else {
                    // a multi-byte char literal ('—') or a lifetime
                    let ch = src[i + 1..].chars().next().map_or(1, char::len_utf8);
                    if b.get(i + 1 + ch) == Some(&b'\'') { i + ch + 2 } else { i + 1 }
                };
                if literals && end > i + 2 {
                    blank(&mut out, i + 1, end - 1);
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("only whole characters were replaced")
}

pub fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

pub fn ident_before(b: &[u8], i: usize) -> bool {
    i > 0 && is_ident(b[i - 1])
}

/// Whether file `rel` is test code as a whole: its parent module's file (`mod.rs`, `lib.rs`) declares it with
/// `#[cfg(test)]` / `#[cfg(all(test, …))]` (`mod tests;`).
pub fn test_file(src_dir: &Path, rel: &str) -> bool {
    let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let Some(stem) = name.strip_suffix(".rs").filter(|s| *s != "mod") else { return false };
    let parent = if dir.is_empty() { "lib.rs".to_string() } else { format!("{dir}/mod.rs") };
    let Ok(text) = std::fs::read_to_string(src_dir.join(parent)) else { return false };
    let s = structure(&text);
    let decl = format!("mod {stem};");
    s.match_indices(&decl).any(|(k, _)| !ident_before(s.as_bytes(), k) && cfg_test_before(&s[..k]))
}

/// The line (1-based) of byte offset `at`, from the offsets where lines start.
pub fn line_of(starts: &[usize], at: usize) -> usize {
    starts.partition_point(|&s| s <= at)
}

/// An inline module (`mod name { … }`): its name, the byte offsets of its braces, and whether it is test code (a
/// `#[cfg(test)]` / `#[cfg(all(test, …))]` attribute on it).
#[derive(Debug, PartialEq)]
pub struct InlineMod {
    pub name: String,
    pub open: usize,
    pub close: usize,
    pub test: bool,
}

/// The inline modules of `structure` (a file with comments and literal contents blanked: `structure()`). A `mod x;`
/// declares a file module and is not one.
pub fn inline_mods(structure: &str) -> Vec<InlineMod> {
    let b = structure.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i..].starts_with(b"mod") || ident_before(b, i) || b.get(i + 3).is_none_or(|&c| !c.is_ascii_whitespace()) {
            i += 1;
            continue;
        }
        let name_at = skip_ws(i + 3);
        let mut e = name_at;
        while e < b.len() && is_ident(b[e]) {
            e += 1;
        }
        let open = skip_ws(e);
        if e == name_at || b.get(open) != Some(&b'{') {
            i = e.max(i + 1);
            continue;
        }
        let mut depth = 0usize;
        let mut close = b.len();
        for (k, &c) in b.iter().enumerate().skip(open) {
            match c {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = k;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(InlineMod { name: structure[name_at..e].to_string(), open, close, test: cfg_test_before(&structure[..i]) });
        // the modules inside it are found as the scan goes on
        i = open + 1;
    }
    out
}

/// Whether the attributes in front of an item (`before` ends where the item's keyword starts) include
/// `#[cfg(test)]` / `#[cfg(all(test, …))]`; a visibility (`pub`, `pub(crate)` …) between them is skipped.
pub fn cfg_test_before(before: &str) -> bool {
    let mut s = before.trim_end();
    let vis = s.strip_suffix(')').and_then(|x| x.rfind("pub(").filter(|&k| x[k + 4..].bytes().all(|c| is_ident(c) || c == b':' || c == b' ')));
    if let Some(k) = vis.filter(|&k| !ident_before(s.as_bytes(), k)) {
        s = s[..k].trim_end();
    } else if let Some(p) = s.strip_suffix("pub").filter(|p| p.as_bytes().last().is_none_or(|&c| !is_ident(c))) {
        s = p.trim_end();
    }
    while let Some(body) = s.strip_suffix(']') {
        // the attribute's `#[`, at bracket depth 0
        let (mut depth, mut start) = (0i32, None);
        for (k, c) in body.char_indices().rev() {
            match c {
                ']' => depth += 1,
                '[' if depth == 0 => {
                    start = Some(k);
                    break;
                }
                '[' => depth -= 1,
                _ => {}
            }
        }
        let Some(k) = start.filter(|&k| body[..k].ends_with('#')) else { return false };
        let attr: String = body[k + 1..].chars().filter(|c| !c.is_whitespace()).collect();
        if attr == "cfg(test)" || attr.starts_with("cfg(all(test,") {
            return true;
        }
        s = body[..k - 1].trim_end();
    }
    false
}

pub fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("read src").map(|e| e.expect("dir entry").path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(root, &p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p.strip_prefix(root).expect("under src").to_string_lossy().replace('\\', "/"));
        }
    }
}
