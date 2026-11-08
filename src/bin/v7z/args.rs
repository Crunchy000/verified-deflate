//! Command-line parsing, in 7-Zip's syntax:
//!
//! ```text
//! v7z <command> [<switches>...] <archive_name> [<file_names>...] [@listfile]
//! ```
//!
//! Commands and switch names are case-insensitive, as in 7-Zip; switch
//! values (paths, names) keep their case. Switches may appear anywhere
//! before `--`.

use std::path::PathBuf;

/// What to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// `a`: add files to an archive (creating it, or updating it).
    Add,
    /// `d`: delete entries from an archive.
    Delete,
    /// `e`: extract, without directory names.
    Extract,
    /// `x`: extract with full paths.
    ExtractFull,
    /// `l`: list an archive.
    List,
    /// `t`: test an archive's integrity.
    Test,
    /// `i`: show the supported formats.
    Info,
}

/// What to do when an extracted file already exists (`-ao`, `-y`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    /// Ask on the terminal (the default).
    Ask,
    /// `-aoa` (and `-y`): overwrite.
    All,
    /// `-aos`: keep the existing file.
    Skip,
    /// `-aou`: extract under a new name (`name_1`).
    RenameNew,
    /// `-aot`: rename the existing file first.
    RenameExisting,
}

/// How file names on the command line match (`-r`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recurse {
    /// `-r-` (the default): a name matches from the archive's root.
    No,
    /// `-r`: a name also matches at any depth.
    Yes,
    /// `-r0`: only wildcard names match at any depth.
    WildcardsOnly,
}

/// A parsed command line.
#[derive(Debug)]
pub struct Args {
    pub command: Command,
    pub archive: Option<String>,
    pub names: Vec<String>,
    /// `-t{Type}`.
    pub kind: Option<String>,
    /// `-mx{N}`.
    pub level: Option<u8>,
    /// `-mm={Method}`.
    pub method: Option<String>,
    /// `-mhc=off`: leave the 7z header uncompressed.
    pub header_compression: bool,
    /// `-o{Dir}`.
    pub out_dir: Option<PathBuf>,
    pub recurse: Recurse,
    pub overwrite: Overwrite,
    /// `-y`: assume yes to every question.
    pub yes: bool,
    /// `-si[{Name}]`: read data (for `a`) or the archive (otherwise) from
    /// standard input.
    pub stdin: Option<String>,
    /// `-so`: write extracted data (or the new archive) to standard output.
    pub stdout: bool,
    /// `-slt`: technical listing.
    pub technical: bool,
    /// `-i`: extra names to include, with their recursion.
    pub include: Vec<(String, Recurse)>,
    /// `-x`: names to exclude, with their recursion.
    pub exclude: Vec<(String, Recurse)>,
    /// `-snl`: store symbolic links as links rather than what they point to.
    pub store_symlinks: bool,
}

/// A command-line error (exit code 7).
#[derive(Debug)]
pub struct UsageError(pub String);

fn read_list_file(path: &str) -> Result<Vec<String>, UsageError> {
    let text = std::fs::read_to_string(path).map_err(|e| UsageError(format!("cannot read list file {path}: {e}")))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
}

/// Parses `-i…` / `-x…` values: `[r[-|0]]{@listfile|!wildcard}`.
fn wildcard_switch(body: &str, default: Recurse) -> Result<Vec<(String, Recurse)>, UsageError> {
    let (recurse, rest) = if let Some(r) = body.strip_prefix("r-").or_else(|| body.strip_prefix("R-")) {
        (Recurse::No, r)
    } else if let Some(r) = body.strip_prefix("r0").or_else(|| body.strip_prefix("R0")) {
        (Recurse::WildcardsOnly, r)
    } else if let Some(r) = body.strip_prefix('r').or_else(|| body.strip_prefix('R')) {
        (Recurse::Yes, r)
    } else {
        (default, body)
    };
    if let Some(w) = rest.strip_prefix('!') {
        Ok(vec![(w.to_string(), recurse)])
    } else if let Some(f) = rest.strip_prefix('@') {
        Ok(read_list_file(f)?.into_iter().map(|n| (n, recurse)).collect())
    } else {
        Err(UsageError(format!("incorrect wildcard switch: {body}")))
    }
}

/// Switches accepted for compatibility that change nothing here (output
/// streams, progress, console charsets, thread counts, timing).
fn ignored(lower: &str) -> bool {
    lower == "bd"
        || lower == "bt"
        || lower.starts_with("bb")
        || lower.starts_with("bso")
        || lower.starts_with("bse")
        || lower.starts_with("bsp")
        || lower.starts_with("scc")
        || lower.starts_with("scs")
        || lower == "ssp"
        || lower == "sse"
        || lower.starts_with("mmt")
}

pub fn parse(argv: &[String]) -> Result<Args, UsageError> {
    let (cmd, rest) = argv.split_first().ok_or_else(|| UsageError("no command".into()))?;
    let command = match cmd.to_ascii_lowercase().as_str() {
        "a" => Command::Add,
        "d" => Command::Delete,
        "e" => Command::Extract,
        "x" => Command::ExtractFull,
        "l" => Command::List,
        "t" => Command::Test,
        "i" => Command::Info,
        other => return Err(UsageError(format!("Unsupported command: {other}"))),
    };
    let mut a = Args {
        command,
        archive: None,
        names: Vec::new(),
        kind: None,
        level: None,
        method: None,
        header_compression: true,
        out_dir: None,
        recurse: Recurse::No,
        overwrite: Overwrite::Ask,
        yes: false,
        stdin: None,
        stdout: false,
        technical: false,
        include: Vec::new(),
        exclude: Vec::new(),
        store_symlinks: false,
    };
    let mut positional = Vec::new();
    let mut switches_done = false;
    for arg in rest {
        if !switches_done && arg == "--" {
            switches_done = true;
            continue;
        }
        if !switches_done && arg.len() > 1 && arg.starts_with('-') {
            let body = &arg[1..];
            let lower = body.to_ascii_lowercase();
            if lower == "y" {
                a.yes = true;
            } else if lower == "r" {
                a.recurse = Recurse::Yes;
            } else if lower == "r-" {
                a.recurse = Recurse::No;
            } else if lower == "r0" {
                a.recurse = Recurse::WildcardsOnly;
            } else if lower == "slt" {
                a.technical = true;
            } else if lower == "so" {
                a.stdout = true;
            } else if lower == "snl" {
                a.store_symlinks = true;
            } else if lower.starts_with("si") {
                a.stdin = Some(body[2..].to_string());
            } else if let Some(mode) = lower.strip_prefix("ao") {
                a.overwrite = match mode {
                    "a" => Overwrite::All,
                    "s" => Overwrite::Skip,
                    "u" => Overwrite::RenameNew,
                    "t" => Overwrite::RenameExisting,
                    _ => return Err(UsageError(format!("incorrect switch: {arg}"))),
                };
            } else if lower.starts_with('o') {
                if body.len() == 1 {
                    return Err(UsageError("-o needs a directory, as in -o/tmp/out".into()));
                }
                a.out_dir = Some(PathBuf::from(&body[1..]));
            } else if let Some(t) = lower.strip_prefix('t') {
                a.kind = Some(t.to_string());
            } else if lower.starts_with('p') {
                return Err(UsageError("encryption (-p) is not supported".into()));
            } else if ignored(&lower) {
            } else if let Some(m) = lower.strip_prefix('m') {
                let level = m.strip_prefix("x=").or_else(|| m.strip_prefix('x'));
                if let Some(n) = level {
                    let n = if n.is_empty() {
                        9
                    } else {
                        n.parse::<u8>().map_err(|_| UsageError(format!("incorrect switch: {arg}")))?
                    };
                    if n > 9 {
                        return Err(UsageError(format!("incorrect compression level: {arg}")));
                    }
                    a.level = Some(n);
                } else if let Some(v) = m.strip_prefix("m=").or_else(|| m.strip_prefix("0=")) {
                    a.method = Some(v.to_string());
                } else if let Some(v) = m.strip_prefix("hc=").or_else(|| m.strip_prefix("hc")) {
                    a.header_compression = match v {
                        "" | "on" | "+" => true,
                        "off" | "-" => false,
                        _ => return Err(UsageError(format!("incorrect switch: {arg}"))),
                    };
                } else if ["s", "f", "qs", "tm", "tc", "ta"].iter().any(|p| m.starts_with(p)) {
                    // Solid mode, filters, sorting and which times to
                    // store: archives are always solid, without filters.
                } else {
                    return Err(UsageError(format!("unsupported method parameter: {arg}")));
                }
            } else if let Some(w) = body.strip_prefix(['i', 'I']) {
                a.include.extend(wildcard_switch(w, Recurse::No)?);
            } else if let Some(w) = body.strip_prefix(['x', 'X']) {
                a.exclude.extend(wildcard_switch(w, Recurse::No)?);
            } else {
                return Err(UsageError(format!("Unknown switch: {arg}")));
            }
            continue;
        }
        if !switches_done && arg.len() > 1 && arg.starts_with('@') {
            if positional.is_empty() {
                return Err(UsageError("the archive name must come before a list file".into()));
            }
            positional.extend(read_list_file(&arg[1..])?);
            continue;
        }
        positional.push(arg.clone());
    }
    if a.yes && a.overwrite == Overwrite::Ask {
        a.overwrite = Overwrite::All;
    }
    let mut positional = positional.into_iter();
    a.archive = positional.next();
    a.names = positional.collect();
    // Reading an archive from standard input (`-si` with x, e, l, t)
    // needs no archive name; everything else but `i` does.
    let needs_archive = command != Command::Info && (command == Command::Add || a.stdin.is_none());
    if needs_archive && a.archive.is_none() {
        return Err(UsageError("Cannot find archive name".into()));
    }
    if command == Command::Delete && a.names.is_empty() && a.include.is_empty() {
        return Err(UsageError("d needs the names of the entries to delete".into()));
    }
    Ok(a)
}
