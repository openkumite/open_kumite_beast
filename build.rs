//! Build stamp: the short git commit (with "+" when the tree has changes) as OKB_GIT for the support report; on
//! Android the C++ runtime linked in statically (cpal's oboe is C++ and its build links none) and the library linked
//! for 16 KiB pages (devices with them exist at target SDK 35; NDKs before r28 default to 4 KiB).
//!
//! The stamp is redone when HEAD, the index, the branch's ref or a tracked file of the build (`src/`, `Cargo.toml`,
//! `Cargo.lock`, build.rs) changes; an edit elsewhere alone (the docs) rebuilds nothing and may leave the "+" stale
//! until the next rebuild. git names the paths (`--git-path`): in a worktree `.git` is a file, HEAD and the index are
//! in the worktree's own git folder and the refs in the shared one. A path that is not there would make cargo rerun
//! this every build, so only the ones there are named; without git (a source archive) only build.rs.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=c++_static");
        println!("cargo:rustc-link-lib=c++abi");
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    }
    // no optional locks: `status` does not rewrite the index (which would rerun this on the next build)
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("--no-optional-locks")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches(['\n', '\r']).to_string())
    };
    let hash = git(&["rev-parse", "--short", "HEAD"]);
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    println!("cargo:rustc-env=OKB_GIT={}{}", hash.as_deref().unwrap_or("unknown"), if dirty { "+" } else { "" });
    println!("cargo:rerun-if-changed=build.rs");
    if hash.is_none() {
        return;
    }
    // the branch HEAD is on (none when detached: then HEAD itself holds the commit)
    let branch = git(&["symbolic-ref", "-q", "HEAD"]);
    let mut names = vec!["HEAD", "index", "packed-refs", "reftable/tables.list", "logs/HEAD"];
    names.extend(branch.as_deref());
    let mut args = vec!["rev-parse"];
    for n in &names {
        args.extend(["--git-path", n]);
    }
    // relative to this folder (the build script's): the git paths as git gives them for a `.git` folder here, and
    // the tracked files of the build (a change makes the tree dirty)
    let here = std::env::current_dir().unwrap_or_default();
    let paths = git(&args).map(|s| s.lines().map(|l| here.join(l)).collect::<Vec<_>>()).unwrap_or_default();
    let files = git(&["ls-files", "-z", "--", "src", "Cargo.toml", "Cargo.lock", "build.rs"]).unwrap_or_default();
    for p in paths.into_iter().chain(files.split('\0').filter(|f| !f.is_empty()).map(|f| here.join(f))) {
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
}
