//! `vzip`: create, extract, list and test ZIP archives with verified-deflate.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::UNIX_EPOCH;

use verified_deflate::zip::{validate_name, DosDateTime, FileOptions, ZipArchive, ZipWriter};
use verified_deflate::{compress_with, decompress, CompressOptions};

const USAGE: &str = "\
vzip - ZIP archives with verified-deflate

usage:
  vzip create [-0..-9] [--no-verify] ARCHIVE.zip PATH...
  vzip extract [-o] [-d DIR] ARCHIVE.zip
  vzip list ARCHIVE.zip
  vzip test ARCHIVE.zip
  vzip deflate [-0..-9] < INPUT > OUTPUT    (raw DEFLATE stream)
  vzip inflate < INPUT > OUTPUT

options:
  -0 .. -9      compression level (default 6; 0 stores)
  --no-verify   skip decompress-and-compare of each compressed entry
  -d DIR        extract into DIR (default: current directory)
  -o            overwrite existing files when extracting";

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let result = match cmd.as_str() {
        "create" | "c" => create(rest),
        "extract" | "x" => extract(rest),
        "list" | "l" => list(rest),
        "test" | "t" => test(rest),
        "deflate" => raw_deflate(rest),
        "inflate" => raw_inflate(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vzip: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Parses `-0`..`-9` flags out of `args`, returning the level and the rest.
fn take_level(args: &[String]) -> (u8, Vec<String>) {
    let mut level = 6;
    let mut rest = Vec::new();
    for a in args {
        let b = a.as_bytes();
        if b.len() == 2 && b[0] == b'-' && b[1].is_ascii_digit() {
            level = b[1] - b'0';
        } else {
            rest.push(a.clone());
        }
    }
    (level, rest)
}

fn create(args: &[String]) -> Result<()> {
    let (level, args) = take_level(args);
    let verify = !args.iter().any(|a| a == "--no-verify");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--no-verify").collect();
    let [archive, paths @ ..] = args.as_slice() else { return Err(USAGE.into()) };
    if paths.is_empty() {
        return Err("nothing to add".into());
    }
    let mut w = ZipWriter::new();
    let base = FileOptions { level, verify, ..FileOptions::default() };
    let (mut files, mut bytes_in) = (0usize, 0u64);
    for p in paths {
        add_path(&mut w, Path::new(p.as_str()), &base, &mut files, &mut bytes_in)?;
    }
    let out = w.finish().map_err(|e| e.to_string())?;
    fs::write(archive.as_str(), &out).map_err(|e| format!("{archive}: {e}"))?;
    let pct = if bytes_in == 0 { 100.0 } else { out.len() as f64 * 100.0 / bytes_in as f64 };
    println!("{archive}: {files} files, {bytes_in} -> {} bytes ({pct:.1}%)", out.len());
    Ok(())
}

/// Archive name for a filesystem path: relative, `/`-separated, with `.`
/// and root components dropped. `..` is refused.
fn archive_name(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str().ok_or_else(|| format!("{path:?}: name is not UTF-8"))?),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => return Err(format!("{}: paths with '..' are not supported", path.display())),
        }
    }
    if parts.is_empty() {
        return Err(format!("{}: no name to store", path.display()));
    }
    Ok(parts.join("/"))
}

fn options_for(meta: &fs::Metadata, base: &FileOptions) -> FileOptions {
    let mut o = *base;
    if let Ok(t) = meta.modified() {
        let secs = match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        };
        o.modified = DosDateTime::from_unix_seconds(secs);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        o.unix_mode = meta.permissions().mode() & 0o7777;
    }
    o
}

fn add_path(w: &mut ZipWriter, path: &Path, base: &FileOptions, files: &mut usize, bytes: &mut u64) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = archive_name(path)?;
    let opts = options_for(&meta, base);
    if meta.is_symlink() {
        eprintln!("vzip: skipping symlink {}", path.display());
    } else if meta.is_dir() {
        w.add_directory(&name, &opts).map_err(|e| e.to_string())?;
        let mut children: Vec<PathBuf> = fs::read_dir(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<_>>()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        children.sort();
        for c in children {
            add_path(w, &c, base, files, bytes)?;
        }
    } else {
        let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        w.add_file(&name, &data, &opts).map_err(|e| format!("{}: {e}", path.display()))?;
        *files += 1;
        *bytes += data.len() as u64;
    }
    Ok(())
}

fn open(args: &[String]) -> Result<Vec<u8>> {
    let [archive] = args else { return Err(USAGE.into()) };
    fs::read(archive).map_err(|e| format!("{archive}: {e}"))
}

fn extract(args: &[String]) -> Result<()> {
    let mut dest = PathBuf::from(".");
    let mut overwrite = false;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-d" => dest = PathBuf::from(it.next().ok_or("-d needs a directory")?),
            "-o" => overwrite = true,
            _ => rest.push(a.clone()),
        }
    }
    let data = open(&rest)?;
    let zip = ZipArchive::parse(&data).map_err(|e| e.to_string())?;
    for (i, e) in zip.entries().iter().enumerate() {
        // Refuse anything that could escape `dest` (absolute paths, `..`).
        validate_name(&e.name).map_err(|err| format!("refusing to extract: {err}"))?;
        let target = dest.join(e.name.trim_end_matches('/'));
        if e.is_dir() {
            fs::create_dir_all(&target).map_err(|err| format!("{}: {err}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        if !overwrite && fs::symlink_metadata(&target).is_ok() {
            return Err(format!("{} exists (use -o to overwrite)", target.display()));
        }
        let bytes = zip.read(i).map_err(|err| format!("{}: {err}", e.name))?;
        fs::write(&target, &bytes).map_err(|err| format!("{}: {err}", target.display()))?;
        #[cfg(unix)]
        if let Some(mode) = e.unix_mode {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode & 0o777));
        }
        println!("  {}", e.name);
    }
    Ok(())
}

fn list(args: &[String]) -> Result<()> {
    let data = open(args)?;
    let zip = ZipArchive::parse(&data).map_err(|e| e.to_string())?;
    println!("{:>10} {:>10} {:>7}  {:<19}  {:>8}  name", "size", "packed", "method", "modified", "crc32");
    let (mut total, mut packed) = (0u64, 0u64);
    for e in zip.entries() {
        let m = e.modified;
        let method = match e.method {
            0 => "stored".to_string(),
            8 => "deflate".to_string(),
            n => n.to_string(),
        };
        println!(
            "{:>10} {:>10} {:>7}  {:04}-{:02}-{:02} {:02}:{:02}:{:02}  {:08x}  {}",
            e.uncompressed_size,
            e.compressed_size,
            method,
            m.year,
            m.month,
            m.day,
            m.hour,
            m.minute,
            m.second,
            e.crc32,
            e.name
        );
        total += u64::from(e.uncompressed_size);
        packed += u64::from(e.compressed_size);
    }
    println!("{total:>10} {packed:>10}  {} entries", zip.len());
    Ok(())
}

fn test(args: &[String]) -> Result<()> {
    let data = open(args)?;
    let zip = ZipArchive::parse(&data).map_err(|e| e.to_string())?;
    let mut bad = 0;
    for (i, e) in zip.entries().iter().enumerate() {
        match zip.read(i) {
            Ok(_) => println!("OK      {}", e.name),
            Err(err) => {
                println!("FAILED  {}: {err}", e.name);
                bad += 1;
            }
        }
    }
    if bad == 0 {
        println!("all {} entries OK", zip.len());
        Ok(())
    } else {
        Err(format!("{bad} of {} entries failed", zip.len()))
    }
}

fn read_stdin() -> Result<Vec<u8>> {
    let mut v = Vec::new();
    io::stdin().read_to_end(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

fn raw_deflate(args: &[String]) -> Result<()> {
    let (level, rest) = take_level(args);
    if !rest.is_empty() {
        return Err(USAGE.into());
    }
    let input = read_stdin()?;
    let out = compress_with(&input, &CompressOptions { level, ..CompressOptions::default() });
    io::stdout().write_all(&out).map_err(|e| e.to_string())
}

fn raw_inflate(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err(USAGE.into());
    }
    let input = read_stdin()?;
    let out = decompress(&input).map_err(|e| e.to_string())?;
    io::stdout().write_all(&out).map_err(|e| e.to_string())
}
