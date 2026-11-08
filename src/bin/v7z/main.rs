//! `v7z`: 7-Zip's command line (`7z`) for the formats verified-deflate
//! handles: zip, tar, gzip and bzip2. The commands `a`, `d`, `e`, `l`, `t` and
//! `x` and the common switches work as in 7-Zip, with 7-Zip's exit codes,
//! so scripts written for `7z` work with `v7z` on these formats.

mod archive;
mod args;
mod paths;

use archive::{Item, ItemKind, Kind, Opened, Time};
use args::{Args, Command, Overwrite, UsageError};
use paths::{destination, link_stays_inside, size_note, through_symlink, Input, InputKind, Selector};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use verified_deflate::gzip_header::{ExtraFlags, FileSystemType, GzBuilder};
use verified_deflate::tar::{TarOptions, TarWriter};
use verified_deflate::zip::{DosDateTime, FileOptions, Method, ZipWriter};
use verified_deflate::{bzip2, gzip};
use verified_deflate::{compress_with, CompressOptions, Strategy};

const BANNER: &str = concat!(
    "\nv7z ",
    env!("CARGO_PKG_VERSION"),
    " (verified-deflate) : the 7-Zip command line for zip, tar, gzip and bzip2\n"
);

const USAGE: &str = "
Usage: v7z <command> [<switches>...] <archive_name> [<file_names>...] [@listfile]

<Commands>
  a : Add files to archive
  d : Delete files from archive
  e : Extract files from archive (without using directory names)
  i : Show information about supported formats
  l : List contents of archive
  t : Test integrity of archive
  x : eXtract files with full paths

<Switches>
  -- : Stop switches and @listfile parsing
  -ao{a|s|t|u} : set Overwrite mode
  -i[r[-|0]]{@listfile|!wildcard} : Include filenames
  -m{Parameters} : set compression Method
    -mx[N] : set compression level: -mx1 (fastest) ... -mx9 (ultra)
    -mm={Copy|Deflate|BZip2} : set method (zip)
  -o{Directory} : set Output directory
  -r[-|0] : Recurse subdirectories for name search
  -si[{name}] : read data from stdin
  -slt : show technical information for l (List) command
  -snl : store symbolic links as links
  -so : write data to stdout
  -t{Type} : Set type of archive (zip, tar, gzip, bzip2)
  -x[r[-|0]]{@listfile|!wildcard} : eXclude filenames
  -y : assume Yes on all queries
";

/// 7-Zip's exit codes.
const EXIT_WARNING: u8 = 1;
const EXIT_FATAL: u8 = 2;
const EXIT_USAGE: u8 = 7;
const EXIT_BREAK: u8 = 255;

/// A failure that ends the command (exit code 2, or 255 if the user quit).
struct Fatal(String, u8);

fn fatal(msg: impl Into<String>) -> Fatal {
    Fatal(msg.into(), EXIT_FATAL)
}

struct Ctx {
    args: Args,
    /// Messages go to stderr when stdout carries data (`-so`).
    log: Box<dyn Write>,
    warnings: Vec<String>,
    errors: usize,
}

impl Ctx {
    fn say(&mut self, s: &str) {
        let _ = self.log.write_all(s.as_bytes());
    }

    fn selector(&self) -> Selector {
        let mut names: Vec<_> = self.args.names.iter().map(|n| (n.clone(), self.args.recurse)).collect();
        names.extend(self.args.include.iter().cloned());
        Selector { names, exclude: self.args.exclude.clone() }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help" | "h" | "--h") {
        print!("{BANNER}{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match args::parse(&argv) {
        Ok(a) => a,
        Err(UsageError(m)) => {
            eprint!("{BANNER}\nCommand Line Error:\n{m}\n");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let log: Box<dyn Write> = if args.stdout { Box::new(io::stderr()) } else { Box::new(io::stdout()) };
    let mut ctx = Ctx { args, log, warnings: Vec::new(), errors: 0 };
    ctx.say(BANNER);
    let result = match ctx.args.command {
        Command::Add => add(&mut ctx),
        Command::Delete => delete(&mut ctx),
        Command::Extract | Command::ExtractFull => extract(&mut ctx),
        Command::List => list(&mut ctx),
        Command::Test => test(&mut ctx),
        Command::Info => {
            info(&mut ctx);
            Ok(())
        }
    };
    let _ = ctx.log.flush();
    // Warnings are always reported; the exit code is the worst outcome.
    if !ctx.warnings.is_empty() {
        eprintln!("\nWARNINGS:");
        for w in &ctx.warnings {
            eprintln!("{w}");
        }
        eprintln!("\n{} WARNING{}", ctx.warnings.len(), if ctx.warnings.len() == 1 { "" } else { "S" });
    }
    match result {
        Err(Fatal(msg, code)) => {
            eprintln!("\nERROR: {msg}");
            ExitCode::from(code)
        }
        Ok(()) if ctx.errors > 0 => ExitCode::from(EXIT_FATAL),
        Ok(()) if !ctx.warnings.is_empty() => ExitCode::from(EXIT_WARNING),
        Ok(()) => ExitCode::SUCCESS,
    }
}

fn info(ctx: &mut Ctx) {
    ctx.say(
        "\nFormats:\n  zip   read write  zip jar   methods Store Deflate BZip2, and Deflate64 (read)\n  \
         tar   read write  tar       ustar, pax, GNU\n  gzip  read write  gz gzip tgz tpz\n  \
         bzip2 read write  bz2 bzip2 tbz2 tbz\n",
    );
}

/// The archive's bytes, from its file or (`-si`) standard input.
fn read_archive(ctx: &Ctx) -> Result<(String, Vec<u8>), Fatal> {
    match (&ctx.args.archive, &ctx.args.stdin) {
        (_, Some(_)) => {
            let mut v = Vec::new();
            io::stdin().read_to_end(&mut v).map_err(|e| fatal(format!("stdin: {e}")))?;
            Ok((ctx.args.archive.clone().unwrap_or_default(), v))
        }
        (Some(name), None) => {
            let data = fs::read(name).map_err(|e| fatal(format!("{name}: {e}")))?;
            Ok((name.clone(), data))
        }
        (None, None) => Err(fatal("Cannot find archive name")),
    }
}

/// The archive's type: `-t`, or else what its bytes show.
fn kind_of(ctx: &Ctx, name: &str, data: &[u8]) -> Result<Kind, Fatal> {
    match &ctx.args.kind {
        Some(t) => Kind::from_type(t).map_err(|m| Fatal(m, EXIT_USAGE)),
        None => Kind::detect(data).ok_or_else(|| fatal(format!("{name}\nCannot open the file as archive"))),
    }
}

fn open<'a>(ctx: &mut Ctx, verb: &str, name: &str, data: &'a [u8]) -> Result<Opened<'a>, Fatal> {
    let kind = kind_of(ctx, name, data)?;
    let opened =
        Opened::open(data, kind, name).map_err(|e| fatal(format!("{name}\nCannot open the file as archive: {e}")))?;
    ctx.say(&format!(
        "\n{verb} archive: {name}\n--\nPath = {name}\nType = {}\nPhysical Size = {}\n\n",
        kind.name(),
        data.len()
    ));
    Ok(opened)
}

fn attr_letters(item: &Item) -> String {
    let a = item.dos_attrib.unwrap_or(0);
    let flag = |bit: u8, c: char| if a & bit != 0 { c } else { '.' };
    format!(
        "{}{}{}{}{}",
        if item.is_dir() { 'D' } else { '.' },
        flag(0x01, 'R'),
        flag(0x02, 'H'),
        flag(0x04, 'S'),
        if item.dos_attrib.is_some() && !item.is_dir() { flag(0x20, 'A') } else { '.' }
    )
}

fn mode_string(item: &Item) -> Option<String> {
    let m = item.mode?;
    let t = match item.kind {
        ItemKind::Dir => 'd',
        ItemKind::Symlink(_) => 'l',
        _ => '-',
    };
    let bits: String = (0..9).map(|i| if m & (0o400 >> i) != 0 { ['r', 'w', 'x'][i % 3] } else { '-' }).collect();
    Some(format!("{t}{bits}"))
}

fn list(ctx: &mut Ctx) -> Result<(), Fatal> {
    let (name, data) = read_archive(ctx)?;
    let opened = open(ctx, "Listing", &name, &data)?;
    let sel = ctx.selector();
    let chosen: Vec<&Item> = opened.items.iter().filter(|i| sel.selects(&i.path)).collect();
    if ctx.args.technical {
        ctx.say("----------\n");
        for item in &chosen {
            let mut s = format!(
                "Path = {}\nFolder = {}\nSize = {}\n",
                item.path,
                if item.is_dir() { '+' } else { '-' },
                item.size.map(|n| n.to_string()).unwrap_or_default()
            );
            if let Some(p) = item.packed {
                s += &format!("Packed Size = {p}\n");
            }
            s += &format!("Modified = {}\n", item.mtime.map(Time::display).unwrap_or_default());
            let letters = attr_letters(item).replace('.', "");
            let attrs = match mode_string(item) {
                Some(m) if letters.is_empty() => m,
                Some(m) => format!("{letters} {m}"),
                None => letters,
            };
            s += &format!("Attributes = {attrs}\n");
            if let Some(c) = item.crc {
                if !item.is_dir() {
                    s += &format!("CRC = {c:08X}\n");
                }
            }
            if let Some(m) = item.method {
                s += &format!("Method = {m}\n");
            }
            if let Some(os) = item.host_os {
                s += &format!("Host OS = {os}\n");
            }
            match &item.kind {
                ItemKind::Symlink(t) => s += &format!("Symbolic Link = {t}\n"),
                ItemKind::HardLink(t) => s += &format!("Hard Link = {t}\n"),
                _ => {}
            }
            if let Some(u) = &item.user {
                s += &format!("User = {u}\n");
            }
            if let Some(g) = &item.group {
                s += &format!("Group = {g}\n");
            }
            ctx.say(&format!("{s}\n"));
        }
        return Ok(());
    }
    let rule = "------------------- ----- ------------ ------------  ------------------------\n";
    let mut s = format!("   Date      Time    Attr         Size   Compressed  Name\n{rule}");
    let (mut files, mut dirs, mut size, mut packed) = (0u64, 0u64, 0u64, 0u64);
    let mut latest: Option<Time> = None;
    for item in &chosen {
        let date = item.mtime.map(Time::display).unwrap_or_else(|| " ".repeat(19));
        let p = item.packed.map(|p| p.to_string()).unwrap_or_default();
        let n = item.size.map(|n| n.to_string()).unwrap_or_default();
        s += &format!("{date} {} {n:>12} {p:>12}  {}\n", attr_letters(item), item.path);
        if item.is_dir() {
            dirs += 1;
        } else {
            files += 1;
            size += item.size.unwrap_or(0);
        }
        packed += item.packed.unwrap_or(0);
        if let Some(t) = item.mtime {
            if latest.is_none_or(|l| t.unix_seconds() > l.unix_seconds()) {
                latest = Some(t);
            }
        }
    }
    let date = latest.map(Time::display).unwrap_or_else(|| " ".repeat(19));
    let counts = if dirs > 0 { format!("{files} files, {dirs} folders") } else { format!("{files} files") };
    s += &format!("{rule}{date}       {size:>12} {packed:>12}  {counts}\n");
    ctx.say(&s);
    Ok(())
}

/// Reads every selected file, as `t` and `x` do, reporting bad ones in
/// 7-Zip's words.
fn check_error(ctx: &mut Ctx, item: &Item, e: &str) {
    let what = if e.contains("CRC") {
        "CRC Failed"
    } else if e.contains("corrupt") || e.contains("Inflate") {
        "Data Error"
    } else {
        e
    };
    ctx.say(&format!("ERROR: {what} : {}\n", item.path));
    ctx.errors += 1;
}

fn summary(ctx: &mut Ctx, dirs: u64, files: u64, size: u64, packed: u64) {
    if ctx.errors > 0 {
        ctx.say(&format!("\nSub items Errors: {}\n\nArchives with Errors: 1\n", ctx.errors));
    } else {
        ctx.say("Everything is Ok\n");
    }
    let mut s = String::new();
    if dirs > 0 {
        s += &format!("\nFolders: {dirs}");
    }
    s += &format!("\nFiles: {files}\nSize:       {size}\nCompressed: {packed}\n");
    ctx.say(&s);
}

fn test(ctx: &mut Ctx) -> Result<(), Fatal> {
    let (name, data) = read_archive(ctx)?;
    let opened = open(ctx, "Testing", &name, &data)?;
    let sel = ctx.selector();
    let (mut dirs, mut files, mut size) = (0, 0, 0);
    for (i, item) in opened.items.iter().enumerate() {
        if !sel.selects(&item.path) {
            continue;
        }
        if item.is_dir() {
            dirs += 1;
            continue;
        }
        files += 1;
        match opened.read(i) {
            Ok(d) => size += d.len() as u64,
            Err(e) => check_error(ctx, item, &e),
        }
    }
    summary(ctx, dirs, files, size, data.len() as u64);
    Ok(())
}

/// What to do about an existing file.
enum Decision {
    Write(PathBuf),
    Skip,
}

fn renamed(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| path.with_file_name(format!("{stem}_{n}{ext}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("some name is free")
}

fn decide(ctx: &mut Ctx, dest: &Path, item: &Item) -> Result<Decision, Fatal> {
    let existing = match fs::symlink_metadata(dest) {
        Err(_) => return Ok(Decision::Write(dest.to_path_buf())),
        Ok(m) => m,
    };
    if existing.is_dir() {
        if item.is_dir() {
            return Ok(Decision::Write(dest.to_path_buf()));
        }
        // A file or link cannot replace a directory (with what is in it).
        ctx.say(&format!("ERROR: Cannot replace a folder with a file : {}\n", item.path));
        ctx.errors += 1;
        return Ok(Decision::Skip);
    }
    loop {
        match ctx.args.overwrite {
            Overwrite::All => return Ok(Decision::Write(dest.to_path_buf())),
            Overwrite::Skip => return Ok(Decision::Skip),
            Overwrite::RenameNew => return Ok(Decision::Write(renamed(dest))),
            Overwrite::RenameExisting => {
                fs::rename(dest, renamed(dest)).map_err(|e| fatal(format!("{}: {e}", dest.display())))?;
                return Ok(Decision::Write(dest.to_path_buf()));
            }
            Overwrite::Ask => {
                eprint!(
                    "\nWould you like to replace the existing file:\n  Path:     {}\nwith the file from archive:\n  Path:     {}\n? (Y)es / (N)o / (A)lways / (S)kip all / A(u)to rename all / (Q)uit? ",
                    dest.display(),
                    item.path
                );
                let mut answer = String::new();
                let n = io::stdin().read_line(&mut answer).unwrap_or(0);
                match answer.trim().to_ascii_lowercase().as_str() {
                    _ if n == 0 => return Err(Fatal("no answer on standard input; use -y or -ao".into(), EXIT_BREAK)),
                    "y" => return Ok(Decision::Write(dest.to_path_buf())),
                    "n" => return Ok(Decision::Skip),
                    "a" => ctx.args.overwrite = Overwrite::All,
                    "s" => ctx.args.overwrite = Overwrite::Skip,
                    "u" => ctx.args.overwrite = Overwrite::RenameNew,
                    "q" => return Err(Fatal("Break signaled".into(), EXIT_BREAK)),
                    _ => {}
                }
            }
        }
    }
}

fn set_times_and_mode(path: &Path, item: &Item) {
    if let Some(t) = item.mtime {
        let secs = t.unix_seconds();
        let time = if secs >= 0 {
            UNIX_EPOCH + Duration::from_secs(secs as u64)
        } else {
            UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
        };
        if let Ok(f) = fs::File::options().write(!item.is_dir()).read(item.is_dir()).open(path) {
            let _ = f.set_modified(time);
        }
    }
    #[cfg(unix)]
    if let Some(mode) = item.mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777));
    }
}

fn extract(ctx: &mut Ctx) -> Result<(), Fatal> {
    let flat = ctx.args.command == Command::Extract;
    let (name, data) = read_archive(ctx)?;
    let opened = open(ctx, "Extracting", &name, &data)?;
    let root = ctx.args.out_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    if !ctx.args.stdout {
        fs::create_dir_all(&root).map_err(|e| fatal(format!("{}: {e}", root.display())))?;
    }
    let sel = ctx.selector();
    let mut stdout = io::stdout().lock();
    let (mut dirs, mut files, mut size) = (0, 0, 0);
    let mut deferred_links = Vec::new();
    let mut made_dirs = Vec::new();
    for (i, item) in opened.items.iter().enumerate() {
        if !sel.selects(&item.path) {
            continue;
        }
        if ctx.args.stdout {
            if item.kind == ItemKind::File {
                files += 1;
                match opened.read(i) {
                    Ok(d) => {
                        size += d.len() as u64;
                        stdout.write_all(&d).map_err(|e| fatal(format!("stdout: {e}")))?;
                    }
                    Err(e) => check_error(ctx, item, &e),
                }
            }
            continue;
        }
        if flat && item.is_dir() {
            dirs += 1;
            continue;
        }
        let Some(dest) = destination(&root, &item.path, flat) else {
            ctx.say(&format!("ERROR: Dangerous path, not extracted : {}\n", item.path));
            ctx.errors += 1;
            continue;
        };
        if through_symlink(&root, &dest) {
            ctx.say(&format!("ERROR: Path through a symbolic link, not extracted : {}\n", item.path));
            ctx.errors += 1;
            continue;
        }
        match &item.kind {
            ItemKind::Dir => {
                dirs += 1;
                if let Err(e) = fs::create_dir_all(&dest) {
                    ctx.say(&format!("ERROR: {e} : {}\n", item.path));
                    ctx.errors += 1;
                } else {
                    made_dirs.push((dest, item));
                }
            }
            ItemKind::File => {
                files += 1;
                let data = match opened.read(i) {
                    Ok(d) => d,
                    Err(e) => {
                        check_error(ctx, item, &e);
                        continue;
                    }
                };
                size += data.len() as u64;
                let Decision::Write(target) = decide(ctx, &dest, item)? else { continue };
                if let Some(parent) = target.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
                    let _ = fs::remove_file(&target);
                }
                match fs::write(&target, &data) {
                    Ok(()) => set_times_and_mode(&target, item),
                    Err(e) => {
                        ctx.say(&format!("ERROR: {e} : {}\n", item.path));
                        ctx.errors += 1;
                    }
                }
            }
            ItemKind::Symlink(_) | ItemKind::HardLink(_) if flat => {}
            ItemKind::Symlink(target) => {
                files += 1;
                let rel = dest.strip_prefix(&root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                if !link_stays_inside(&rel, target) {
                    ctx.warnings.push(format!("Dangerous symbolic link path was ignored : {} -> {target}", item.path));
                    continue;
                }
                deferred_links.push((dest, item, target.clone(), false));
            }
            ItemKind::HardLink(target) => {
                files += 1;
                match destination(&root, target, false) {
                    Some(t) => deferred_links.push((dest, item, t.to_string_lossy().into_owned(), true)),
                    None => {
                        ctx.warnings.push(format!("Dangerous hard link path was ignored : {} -> {target}", item.path))
                    }
                }
            }
            ItemKind::Special => {}
        }
    }
    // Links last, so no file is written through one.
    for (dest, item, target, hard) in deferred_links {
        let Decision::Write(path) = decide(ctx, &dest, item)? else { continue };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::symlink_metadata(&path).is_ok() {
            let _ = fs::remove_file(&path);
        }
        let r = if hard {
            fs::hard_link(&target, &path).or_else(|_| fs::copy(&target, &path).map(|_| ()))
        } else {
            make_symlink(&target, &path)
        };
        if let Err(e) = r {
            ctx.say(&format!("ERROR: {e} : {}\n", item.path));
            ctx.errors += 1;
        }
    }
    // Directory times last, after their contents were written.
    for (dir, item) in made_dirs.iter().rev() {
        set_times_and_mode(dir, item);
    }
    summary(ctx, dirs, files, size, data.len() as u64);
    Ok(())
}

#[cfg(unix)]
fn make_symlink(target: &str, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(not(unix))]
fn make_symlink(target: &str, path: &Path) -> io::Result<()> {
    // Without symbolic links, store the target as the file's contents, as
    // 7-Zip does when it cannot create a link.
    fs::write(path, target)
}

/// The compression level from `-mx` (7-Zip's default is 5) and the
/// method from `-mm`. A bzip2 level is the block size in 100 kB, which
/// 7-Zip sets from `-mx` as 1 (`-mx1`, `-mx2`), 5 (`-mx3`, `-mx4`) or 9.
fn compression(ctx: &Ctx, kind: Kind) -> Result<(u8, Method), Fatal> {
    let level = ctx.args.level.unwrap_or(5);
    let bzip2_level = match level {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 5,
        _ => 9,
    };
    match (kind, ctx.args.method.as_deref().map(str::to_ascii_lowercase).as_deref()) {
        (_, Some("deflate64")) => Err(Fatal("writing Deflate64 is not supported (it can be read)".into(), EXIT_USAGE)),
        (Kind::Zip | Kind::Gzip | Kind::Tar, None | Some("deflate")) => Ok((level, Method::Deflate)),
        (Kind::Zip, Some("copy")) => Ok((0, Method::Deflate)),
        (Kind::Zip, Some("bzip2")) => Ok((bzip2_level, Method::Bzip2)),
        (Kind::Bzip2, None | Some("bzip2")) => Ok((bzip2_level.max(1), Method::Bzip2)),
        (_, Some(m)) => Err(Fatal(format!("method {m} is not supported for {}", kind.name()), EXIT_USAGE)),
    }
}

fn mtime_of(meta: &fs::Metadata) -> i64 {
    match meta.modified().ok().map(|t| t.duration_since(UNIX_EPOCH)) {
        Some(Ok(d)) => d.as_secs() as i64,
        Some(Err(e)) => -(e.duration().as_secs() as i64),
        None => 0,
    }
}

#[cfg(unix)]
fn unix_meta(meta: &fs::Metadata) -> (u32, u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.mode() & 0o7777, u64::from(meta.uid()), u64::from(meta.gid()))
}

#[cfg(not(unix))]
fn unix_meta(meta: &fs::Metadata) -> (u32, u64, u64) {
    (if meta.is_dir() { 0o755 } else { 0o644 }, 0, 0)
}

/// Names stored in an archive compare without a trailing `/`.
fn same_name(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

fn add(ctx: &mut Ctx) -> Result<(), Fatal> {
    let archive = ctx.args.archive.clone().ok_or_else(|| fatal("Cannot find archive name"))?;
    let kind = match &ctx.args.kind {
        Some(t) => Kind::from_type(t).map_err(|m| Fatal(m, EXIT_USAGE))?,
        None => Kind::from_extension(&archive).ok_or_else(|| {
            Fatal(
                format!("{archive}: the 7z format is not supported yet; name the archive .zip, .tar, .gz or .bz2, or use -t"),
                EXIT_USAGE,
            )
        })?,
    };
    let (level, method) = compression(ctx, kind)?;
    let exclude = Selector { names: Vec::new(), exclude: ctx.args.exclude.clone() };
    // The inputs: files from disk, or (-si) standard input.
    let mut stdin_data = None;
    let inputs: Vec<Input> = if let Some(name) = ctx.args.stdin.clone() {
        let mut v = Vec::new();
        io::stdin().read_to_end(&mut v).map_err(|e| fatal(format!("stdin: {e}")))?;
        stdin_data = Some((name, v));
        Vec::new()
    } else {
        if ctx.args.names.is_empty() {
            return Err(Fatal("a needs the names of the files to add".into(), EXIT_USAGE));
        }
        ctx.say("\nScanning the drive:\n");
        let mut warnings = Vec::new();
        let found = paths::collect(
            &ctx.args.names,
            ctx.args.recurse,
            ctx.args.store_symlinks,
            &exclude,
            Some(Path::new(&archive)),
            &mut warnings,
        );
        ctx.warnings.extend(warnings);
        let dirs = found.iter().filter(|i| i.kind == InputKind::Dir).count();
        let bytes: u64 = found.iter().filter(|i| i.kind == InputKind::File).map(|i| i.meta.len()).sum();
        ctx.say(&format!("{dirs} folders, {} files, {}\n", found.len() - dirs, size_note(bytes)));
        found
    };
    // An existing archive is updated: its entries are kept unless replaced.
    let existing = if !ctx.args.stdout && Path::new(&archive).exists() {
        Some(fs::read(&archive).map_err(|e| fatal(format!("{archive}: {e}")))?)
    } else {
        None
    };
    let old = match &existing {
        Some(data) if !matches!(kind, Kind::Gzip | Kind::Bzip2) => {
            let opened = open(ctx, "Open", &archive, data)?;
            if opened.kind != kind {
                return Err(fatal(format!("{archive} is a {} archive, not {}", opened.kind.name(), kind.name())));
            }
            Some(opened)
        }
        _ => None,
    };
    ctx.say(&format!("\n{} archive: {archive}\n\n", if old.is_some() { "Updating" } else { "Creating" }));
    let new_names: Vec<String> = match &stdin_data {
        Some((n, _)) => vec![n.clone()],
        None => inputs.iter().map(|i| i.name.clone()).collect(),
    };
    let bytes = match kind {
        Kind::Zip => build_zip(ctx, old.as_ref(), &inputs, stdin_data.as_ref(), &new_names, level, method)?,
        Kind::Tar => build_tar(ctx, old.as_ref(), &inputs, stdin_data.as_ref(), &new_names)?,
        Kind::Gzip => build_gzip(ctx, &inputs, stdin_data.as_ref(), level)?,
        Kind::Bzip2 => {
            let (_, data, _) = single_file(ctx, kind, &inputs, stdin_data.as_ref())?;
            bzip2::compress(&data, level)
        }
    };
    write_archive(ctx, &archive, &bytes)?;
    let read = if stdin_data.is_some() { 1 } else { inputs.iter().filter(|i| i.kind == InputKind::File).count() };
    ctx.say(&format!(
        "\nFiles read from disk: {read}\nArchive size: {}\nEverything is Ok\n",
        size_note(bytes.len() as u64)
    ));
    Ok(())
}

fn write_archive(ctx: &mut Ctx, archive: &str, bytes: &[u8]) -> Result<(), Fatal> {
    if ctx.args.stdout {
        return io::stdout().write_all(bytes).map_err(|e| fatal(format!("stdout: {e}")));
    }
    // Write beside the archive, then rename over it, so a failure leaves
    // the old archive as it was.
    let tmp = format!("{archive}.v7z-{}.tmp", std::process::id());
    fs::write(&tmp, bytes).map_err(|e| fatal(format!("{tmp}: {e}")))?;
    fs::rename(&tmp, archive).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        fatal(format!("{archive}: {e}"))
    })
}

fn read_input(ctx: &mut Ctx, input: &Input) -> Option<Vec<u8>> {
    match fs::read(&input.disk) {
        Ok(d) => Some(d),
        Err(e) => {
            ctx.warnings.push(format!("{} : {e}", input.disk.display()));
            None
        }
    }
}

fn build_zip(
    ctx: &mut Ctx,
    old: Option<&Opened>,
    inputs: &[Input],
    stdin: Option<&(String, Vec<u8>)>,
    new_names: &[String],
    level: u8,
    method: Method,
) -> Result<Vec<u8>, Fatal> {
    let mut w = ZipWriter::new();
    if let Some(zip) = old.and_then(Opened::zip) {
        for (i, e) in zip.entries().iter().enumerate() {
            if !new_names.iter().any(|n| same_name(n, &e.name)) {
                w.copy_entry(zip, i).map_err(|e| fatal(e.to_string()))?;
            }
        }
    }
    let opts = |meta: Option<&fs::Metadata>| {
        let (mode, mtime) = match meta {
            Some(m) => (unix_meta(m).0, mtime_of(m)),
            None => (0o644, SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)),
        };
        FileOptions {
            level,
            method,
            strategy: Strategy::Auto,
            modified: DosDateTime::from_unix_seconds(mtime),
            unix_mode: mode,
            verify: true,
        }
    };
    if let Some((name, data)) = stdin {
        if name.is_empty() {
            return Err(Fatal("-si needs a name for a zip entry, as in -sidata.txt".into(), EXIT_USAGE));
        }
        w.add_file(name, data, &opts(None)).map_err(|e| fatal(e.to_string()))?;
    }
    for input in inputs {
        let o = opts(Some(&input.meta));
        let r = match &input.kind {
            InputKind::Dir => w.add_directory(&input.name, &o),
            InputKind::Symlink(t) => w.add_symlink(&input.name, t, &o),
            InputKind::File => match read_input(ctx, input) {
                Some(data) => w.add_file(&input.name, &data, &o),
                None => continue,
            },
        };
        if let Err(e) = r {
            ctx.warnings.push(format!("{} : {e}", input.name));
        }
    }
    w.finish().map_err(|e| fatal(e.to_string()))
}

fn build_tar(
    ctx: &mut Ctx,
    old: Option<&Opened>,
    inputs: &[Input],
    stdin: Option<&(String, Vec<u8>)>,
    new_names: &[String],
) -> Result<Vec<u8>, Fatal> {
    let mut w = TarWriter::new();
    if let Some(tar) = old.and_then(Opened::tar) {
        for (i, e) in tar.entries().iter().enumerate() {
            if !new_names.iter().any(|n| same_name(n, &e.path_lossy())) {
                w.copy_entry(tar, i);
            }
        }
    }
    if let Some((name, data)) = stdin {
        if name.is_empty() {
            return Err(Fatal("-si needs a name for a tar entry, as in -sidata.txt".into(), EXIT_USAGE));
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        w.add_file(name, data, &TarOptions { mode: 0o644, mtime: now, ..TarOptions::default() })
            .map_err(|e| fatal(e.to_string()))?;
    }
    for input in inputs {
        let (mode, uid, gid) = unix_meta(&input.meta);
        let o = TarOptions { mode, mtime: mtime_of(&input.meta), uid, gid, ..TarOptions::default() };
        let r = match &input.kind {
            InputKind::Dir => w.add_directory(&input.name, &o),
            InputKind::Symlink(t) => w.add_symlink(&input.name, t, &o),
            InputKind::File => match read_input(ctx, input) {
                Some(data) => w.add_file(&input.name, &data, &o),
                None => continue,
            },
        };
        if let Err(e) = r {
            ctx.warnings.push(format!("{} : {e}", input.name));
        }
    }
    Ok(w.finish())
}

/// The one file a gzip or bzip2 archive holds: its name (without
/// directories), data and modification time.
fn single_file(
    ctx: &mut Ctx,
    kind: Kind,
    inputs: &[Input],
    stdin: Option<&(String, Vec<u8>)>,
) -> Result<(String, Vec<u8>, i64), Fatal> {
    let files: Vec<&Input> = inputs.iter().filter(|i| i.kind == InputKind::File).collect();
    match (stdin, files.as_slice()) {
        (Some((name, data)), []) => Ok((name.clone(), data.clone(), 0)),
        (None, [one]) if inputs.len() == 1 => {
            let data = read_input(ctx, one).ok_or_else(|| fatal(format!("cannot read {}", one.disk.display())))?;
            let base = one.name.rsplit('/').next().unwrap_or(&one.name).to_string();
            Ok((base, data, mtime_of(&one.meta)))
        }
        _ => Err(Fatal(format!("a {} archive holds exactly one file", kind.name()), EXIT_USAGE)),
    }
}

fn build_gzip(ctx: &mut Ctx, inputs: &[Input], stdin: Option<&(String, Vec<u8>)>, level: u8) -> Result<Vec<u8>, Fatal> {
    let (name, data, mtime) = single_file(ctx, Kind::Gzip, inputs, stdin)?;
    let mut builder = GzBuilder::new().mtime(u32::try_from(mtime).unwrap_or(0)).os(FileSystemType::Unix);
    if !name.is_empty() {
        builder = builder.filename(name.as_bytes());
    }
    builder = builder.xfl(match level {
        9 => ExtraFlags::MaximumCompression,
        1 => ExtraFlags::FastestCompression,
        _ => ExtraFlags::Default,
    });
    let mut out = builder.into_header();
    out.extend_from_slice(&compress_with(&data, &CompressOptions { level, strategy: Strategy::Auto, verify: true }));
    out.extend_from_slice(&gzip::trailer(&data));
    Ok(out)
}

fn delete(ctx: &mut Ctx) -> Result<(), Fatal> {
    let archive = ctx.args.archive.clone().ok_or_else(|| fatal("Cannot find archive name"))?;
    let data = fs::read(&archive).map_err(|e| fatal(format!("{archive}: {e}")))?;
    let opened = open(ctx, "Open", &archive, &data)?;
    let sel = ctx.selector();
    let doomed: Vec<bool> = opened.items.iter().map(|i| sel.selects(&i.path)).collect();
    let n = doomed.iter().filter(|&&d| d).count();
    let bytes = match opened.kind {
        Kind::Zip => {
            let zip = opened.zip().expect("a zip archive");
            let mut w = ZipWriter::new();
            for i in (0..zip.len()).filter(|&i| !doomed[i]) {
                w.copy_entry(zip, i).map_err(|e| fatal(e.to_string()))?;
            }
            w.finish().map_err(|e| fatal(e.to_string()))?
        }
        Kind::Tar => {
            let tar = opened.tar().expect("a tar archive");
            let mut w = TarWriter::new();
            for i in (0..tar.len()).filter(|&i| !doomed[i]) {
                w.copy_entry(tar, i);
            }
            w.finish()
        }
        Kind::Gzip | Kind::Bzip2 => {
            return Err(fatal(format!("{} archives do not support deleting", opened.kind.name())))
        }
    };
    ctx.say(&format!("Updating archive: {archive}\n\nDelete data from archive: {n} files\n"));
    if n > 0 {
        write_archive(ctx, &archive, &bytes)?;
    }
    ctx.say(&format!(
        "\nArchive size: {}\nEverything is Ok\n",
        size_note(if n > 0 { bytes.len() } else { data.len() } as u64)
    ));
    Ok(())
}
