#!/usr/bin/env python3
"""Turns the unit-test modules of deflate 1.0 (src/lib.rs, src/writer.rs)
into an integration test that runs against verified-deflate imported as
`deflate`.

Only references to deflate's private items are rewritten: its test helpers
(replaced by the helpers below), a private module path, and two writer tests
that read the encoder's private output buffer (they read the same bytes
through a shared writer instead). Usage: port.py <deflate-rs dir> <out.rs>
"""
import sys

src_dir, out_path = sys.argv[1], sys.argv[2]


def test_module(path):
    src = open(path).read()
    i = src.index('#[cfg(test)]\nmod test {')
    j = src.index('{', i) + 1
    depth, k = 1, j
    while depth:
        depth += {'{': 1, '}': -1}.get(src[k], 0)
        k += 1
    return src[j:k - 1]


def replace(s, old, new, count=-1):
    if old not in s:
        sys.exit('port.py: expected text not found (has deflate changed?): %r' % old[:60])
    return s.replace(old, new, count)


lib = test_module(src_dir + '/src/lib.rs')
lib = replace(lib, '    use super::*;\n', '')
lib = replace(lib, '    #[cfg(feature = "gzip")]\n    use test_utils::decompress_gzip;\n', '')
lib = replace(lib, '    use test_utils::{decompress_to_end, decompress_zlib, get_test_data};\n', '')
lib = replace(lib, 'use input_buffer::BUFFER_SIZE;', 'const BUFFER_SIZE: usize = 32768 * 2 + 258;')
lib = replace(lib, 'compression_options::CompressionOptions::default()', 'CompressionOptions::default()')

wr = test_module(src_dir + '/src/writer.rs')
wr = replace(wr, '    use super::*;\n', '')
wr = replace(wr, '    use crate::compression_options::CompressionOptions;\n', '')
wr = replace(wr, '    use crate::test_utils::{decompress_to_end, decompress_zlib, get_test_data};\n', '')
wr = replace(wr, 'use crate::compression_options::Compression;', 'use deflate::Compression;')
for name in ['fn writer_sync()', 'fn writer_sync_multiple()']:
    i = wr.index(name)
    j = wr.find('    #[test]', i)
    j = len(wr) if j < 0 else j
    seg = wr[i:j]
    seg = replace(seg, 'let compressed = {', 'let shared = Shared::default();\n        let compressed = {', 1)
    seg = replace(seg, 'DeflateEncoder::new(\n                Vec::with_capacity(data.len() / 3),',
                  'DeflateEncoder::new(\n                shared.clone(),', 1)
    seg = replace(seg, 'let buf = &mut compressor.deflate_state.inner.as_mut().unwrap();',
                  'let buf = shared.0.borrow();')
    seg = replace(seg, 'compressor.finish().unwrap()\n        };',
                  'compressor.finish().unwrap();\n            let v = shared.0.borrow().clone();\n            v\n        };', 1)
    wr = wr[:i] + seg + wr[j:]

PRELUDE = '''//! deflate 1.0's own unit tests (src/lib.rs and src/writer.rs), generated
//! by tools/deflate-upstream/port.py, run against verified-deflate imported
//! as `deflate`.
#![allow(unused_imports, dead_code, clippy::all)]
extern crate deflate;
use deflate::gzip_header::GzBuilder;
use deflate::write::*;
use deflate::*;
use std::io::Write;

fn get_test_data() -> Vec<u8> {
    std::fs::read("tests/pg11.txt").unwrap()
}
fn decompress_to_end(input: &[u8]) -> Vec<u8> {
    deflate::decompress(input).expect("Decompression failed!")
}
fn decompress_zlib(c: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec_zlib(c).expect("Decompression failed!")
}

/// A minimal gzip reader standing in for gzip_header::read_gz_header plus
/// decompression: checks the CRC-32 and length trailer.
struct Header {
    comment: Option<Vec<u8>>,
}
impl Header {
    fn comment(&self) -> Option<&[u8]> {
        self.comment.as_deref()
    }
}
fn decompress_gzip(c: &[u8]) -> (Header, Vec<u8>) {
    assert_eq!(&c[..3], &[0x1f, 0x8b, 8]);
    let flg = c[3];
    let mut p = 10;
    if flg & 4 != 0 {
        p += 2 + u16::from_le_bytes([c[p], c[p + 1]]) as usize;
    }
    if flg & 8 != 0 {
        while c[p] != 0 {
            p += 1;
        }
        p += 1;
    }
    let mut comment = None;
    if flg & 16 != 0 {
        let s = p;
        while c[p] != 0 {
            p += 1;
        }
        comment = Some(c[s..p].to_vec());
        p += 1;
    }
    if flg & 2 != 0 {
        p += 2;
    }
    let d = deflate::decompress_with_limit(&c[p..], usize::MAX).unwrap();
    let t = &c[p + d.consumed..];
    assert_eq!(u32::from_le_bytes([t[0], t[1], t[2], t[3]]), deflate::crc32(&d.data));
    assert_eq!(u32::from_le_bytes([t[4], t[5], t[6], t[7]]), d.data.len() as u32);
    (Header { comment }, d.data)
}

/// A writer whose contents can be read while an encoder owns it.
#[derive(Clone, Default)]
struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
'''

with open(out_path, 'w') as f:
    f.write(PRELUDE)
    f.write('\nmod from_lib {\n    use super::*;\n' + lib + '\n}\n')
    f.write('\nmod from_writer {\n    use super::*;\n' + wr + '\n}\n')
