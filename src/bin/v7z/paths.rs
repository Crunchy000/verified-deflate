//! File names: selecting archive items by command-line names (7-Zip's
//! wildcard rules), finding the files to add, and placing extracted files
//! where they cannot escape the output directory.

use crate::args::Recurse;
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// `*` (any run of characters) and `?` (one character) match of one path
/// component.
pub fn wildcard(pat: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), s.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn has_wildcard(s: &str) -> bool {
    s.contains(['*', '?'])
}

/// Whether item `path` is selected by `pattern`: the pattern's components
/// match the item's first components (so naming a directory selects
/// everything in it), or, with recursion, components starting at any
/// depth.
pub fn matches(pattern: &str, path: &str, recurse: Recurse) -> bool {
    let pat: Vec<&str> = pattern.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".").collect();
    let item: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if pat.is_empty() {
        return true;
    }
    let deep = match recurse {
        Recurse::Yes => true,
        Recurse::No => false,
        Recurse::WildcardsOnly => has_wildcard(pattern),
    };
    let last_start = if deep { item.len() } else { 1 };
    (0..last_start.min(item.len()))
        .any(|o| o + pat.len() <= item.len() && pat.iter().zip(&item[o..]).all(|(p, c)| wildcard(p, c)))
}

/// Which items a command applies to: those matching a name (all of them if
/// no names were given) and no exclusion.
pub struct Selector {
    pub names: Vec<(String, Recurse)>,
    pub exclude: Vec<(String, Recurse)>,
}

impl Selector {
    pub fn selects(&self, path: &str) -> bool {
        (self.names.is_empty() || self.names.iter().any(|(p, r)| matches(p, path, *r)))
            && !self.exclude.iter().any(|(p, r)| matches(p, path, *r))
    }
}

/// What an input file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    File,
    Dir,
    /// A symbolic link stored as a link (`-snl`), to this target.
    Symlink(String),
}

/// A file to add.
#[derive(Debug)]
pub struct Input {
    pub disk: PathBuf,
    /// Its name in the archive.
    pub name: String,
    pub kind: InputKind,
    pub meta: fs::Metadata,
}

/// The archive name for a command-line path: relative paths keep their
/// components (without `.` and `..`); absolute ones keep only their last.
fn archive_name(arg: &Path) -> String {
    if arg.is_absolute() || arg.components().any(|c| c == Component::ParentDir) {
        return arg.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    }
    arg.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Collects the files named on the command line for `a`. Directories are
/// added with everything in them; a wildcard in the last component is
/// expanded in its directory (and below it with `-r`). Problems become
/// warnings. `skip` is the archive being written, never added to itself.
pub fn collect(
    names: &[String],
    recurse: Recurse,
    store_symlinks: bool,
    exclude: &Selector,
    skip: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Vec<Input> {
    let skip = skip.and_then(|p| fs::canonicalize(p).ok());
    let mut c = Collector { out: Vec::new(), seen: HashSet::new(), store_symlinks, exclude, skip, warnings };
    for arg in names {
        let path = Path::new(arg.trim_end_matches('/'));
        let path = if path.as_os_str().is_empty() { Path::new("/") } else { path };
        let last = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if has_wildcard(&last) {
            let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
            let prefix = archive_name(dir);
            let deep = matches!(recurse, Recurse::Yes | Recurse::WildcardsOnly);
            c.expand(dir, &prefix, &last, deep);
        } else {
            let name = archive_name(path);
            if name.is_empty() && path.is_dir() {
                // `.` (or `/`): the directory's contents, not the directory.
                for (child, child_name) in c.sorted_dir(path) {
                    c.add(&child, child_name);
                }
            } else {
                c.add(path, if name.is_empty() { last } else { name });
            }
        }
    }
    c.out
}

struct Collector<'a> {
    out: Vec<Input>,
    seen: HashSet<PathBuf>,
    store_symlinks: bool,
    exclude: &'a Selector,
    skip: Option<PathBuf>,
    warnings: &'a mut Vec<String>,
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

impl Collector<'_> {
    fn sorted_dir(&mut self, dir: &Path) -> Vec<(PathBuf, String)> {
        match fs::read_dir(dir) {
            Ok(rd) => {
                let mut v: Vec<_> = rd
                    .filter_map(Result::ok)
                    .map(|e| (e.path(), e.file_name().to_string_lossy().into_owned()))
                    .collect();
                v.sort_by(|a, b| a.1.cmp(&b.1));
                v
            }
            Err(e) => {
                self.warnings.push(format!("{}: {e}", dir.display()));
                Vec::new()
            }
        }
    }

    fn expand(&mut self, dir: &Path, prefix: &str, pattern: &str, deep: bool) {
        for (path, name) in self.sorted_dir(dir) {
            if wildcard(pattern, &name) {
                self.add(&path, join(prefix, &name));
            } else if deep && path.is_dir() && !path.is_symlink() {
                self.expand(&path, &join(prefix, &name), pattern, deep);
            }
        }
    }

    fn add(&mut self, path: &Path, name: String) {
        if self.exclude.selects_excluded(&name) {
            return;
        }
        let link = fs::symlink_metadata(path);
        let meta = match (&link, self.store_symlinks) {
            (Ok(m), true) if m.file_type().is_symlink() => {
                let target = fs::read_link(path).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
                self.out.push(Input {
                    disk: path.to_path_buf(),
                    name,
                    kind: InputKind::Symlink(target),
                    meta: m.clone(),
                });
                return;
            }
            _ => fs::metadata(path),
        };
        let meta = match meta {
            Ok(m) => m,
            Err(e) => {
                self.warnings.push(format!("{}: {e}", path.display()));
                return;
            }
        };
        if let (Some(skip), Ok(canon)) = (&self.skip, fs::canonicalize(path)) {
            if &canon == skip {
                return;
            }
        }
        if meta.is_dir() {
            // Following links to directories could loop; each directory is
            // entered once.
            let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !self.seen.insert(canon) {
                return;
            }
            self.out.push(Input { disk: path.to_path_buf(), name: name.clone(), kind: InputKind::Dir, meta });
            for (child, child_name) in self.sorted_dir(path) {
                self.add(&child, join(&name, &child_name));
            }
        } else {
            self.out.push(Input { disk: path.to_path_buf(), name, kind: InputKind::File, meta });
        }
    }
}

impl Selector {
    /// Whether an exclusion matches (for inputs, where there are no names).
    fn selects_excluded(&self, path: &str) -> bool {
        self.exclude.iter().any(|(p, r)| matches(p, path, *r))
    }
}

/// Where an item extracts to under `root`: its path made safe (no
/// absolute path, no `..`), or just its last component for `e`. `None` if
/// nothing safe is left.
pub fn destination(root: &Path, item_path: &str, flat: bool) -> Option<PathBuf> {
    let safe = verified_deflate::tar::safe_path(item_path.as_bytes())?;
    let rel = if flat { safe.rsplit('/').next()?.to_string() } else { safe };
    Some(root.join(rel))
}

/// True if a component of `path` between `root` and `path` itself is a
/// symbolic link. Files are never written through one, so a link in the
/// archive cannot redirect later entries outside `root`.
pub fn through_symlink(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else { return true };
    let mut p = root.to_path_buf();
    let parts: Vec<_> = rel.components().collect();
    for c in &parts[..parts.len().saturating_sub(1)] {
        p.push(c);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

/// True if a symbolic link at `link` (a safe relative path) to `target`
/// stays inside the output directory, read lexically: not absolute, and
/// never climbing above the root.
pub fn link_stays_inside(link: &str, target: &str) -> bool {
    if target.starts_with('/') || target.is_empty() {
        return false;
    }
    let mut depth = link.split('/').filter(|c| !c.is_empty()).count() as i64 - 1;
    for c in target.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

/// A byte count as 7-Zip prints one: `1234 bytes (2 KiB)`.
pub fn size_note(n: u64) -> String {
    let (unit, div) = if n < 1 << 20 {
        ("KiB", 1u64 << 10)
    } else if n < 1 << 30 {
        ("MiB", 1 << 20)
    } else {
        ("GiB", 1 << 30)
    };
    format!("{n} bytes ({} {unit})", n.div_ceil(div))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_like_7zip() {
        assert!(wildcard("*.txt", "a.txt"));
        assert!(wildcard("a?c", "abc"));
        assert!(!wildcard("*.txt", "a.txt.gz"));
        assert!(wildcard("*", ""));
        assert!(wildcard("a*b*c", "aXbYbZc"));
        assert!(matches("*.txt", "a.txt", Recurse::No));
        assert!(!matches("*.txt", "d/a.txt", Recurse::No));
        assert!(matches("*.txt", "d/a.txt", Recurse::Yes));
        assert!(matches("d", "d/e/f", Recurse::No));
        assert!(matches("d/e", "d/e/f", Recurse::No));
        assert!(!matches("e", "d/e/f", Recurse::No));
        assert!(matches("e", "d/e/f", Recurse::Yes));
        assert!(!matches("e", "d/e/f", Recurse::WildcardsOnly));
        assert!(matches("*", "d/e/f", Recurse::No));
    }

    #[test]
    fn links_are_confined() {
        assert!(link_stays_inside("a/link", "b"));
        assert!(link_stays_inside("a/link", "../b"));
        assert!(!link_stays_inside("a/link", "../../b"));
        assert!(!link_stays_inside("link", "/etc/passwd"));
        assert!(link_stays_inside("a/b/link", "../../x/./y"));
        assert!(!link_stays_inside("link", ".."));
    }

    #[test]
    fn destinations_stay_inside() {
        let root = Path::new("/out");
        assert_eq!(destination(root, "a/b", false), Some(PathBuf::from("/out/a/b")));
        assert_eq!(destination(root, "/etc/passwd", false), Some(PathBuf::from("/out/etc/passwd")));
        assert_eq!(destination(root, "a/../../x", false), None);
        assert_eq!(destination(root, "a/b", true), Some(PathBuf::from("/out/b")));
    }

    #[test]
    fn sizes_print_like_7zip() {
        assert_eq!(size_note(1234), "1234 bytes (2 KiB)");
        assert_eq!(size_note(3 << 20), "3145728 bytes (3 MiB)");
    }
}
