//! Encoders implementing [`std::io::Write`], with the API of the `deflate`
//! crate's `write` module: [`DeflateEncoder`] (raw DEFLATE),
//! [`ZlibEncoder`] (RFC 1950) and [`GzEncoder`] (RFC 1952).
//!
//! Input is buffered and compressed in chunks of 256 KiB, each able to
//! refer back 32 KiB into earlier input, so memory use stays bounded for
//! streams of any length. `flush` performs a zlib-style sync flush: all
//! input so far is compressed and the output ends with an empty stored
//! block, so whatever has been written decodes to all the input so far.
//!
//! Dropping an encoder finishes the stream, ignoring write errors; call
//! `finish` to see them.

use crate::adler32::Adler32;
use crate::compat::{CompressionOptions, ZLIB_HEADER};
use crate::crc32::Crc32;
use crate::deflate::StreamEncoder;
use crate::gzip_header::GzBuilder;
use std::io::{self, Write};
use std::thread;

const ERR_STR: &str = "the wrapped writer is missing; this is a bug";

/// The part shared by the three encoders: the compressor, output not yet
/// handed to the writer, and the writer.
struct Core<W: Write> {
    enc: StreamEncoder,
    /// Bytes produced but not yet accepted by the writer.
    out: Vec<u8>,
    /// How much of `out` the writer has accepted.
    sent: usize,
    /// The final block has been written (no more input is accepted until a
    /// reset).
    finished: bool,
    inner: Option<W>,
}

impl<W: Write> Core<W> {
    fn new(writer: W, options: CompressionOptions, prefix: Vec<u8>) -> Self {
        let (cfg, strategy) = options.engine();
        Self { enc: StreamEncoder::new(cfg, strategy), out: prefix, sent: 0, finished: false, inner: Some(writer) }
    }

    /// Moves the compressor's finished bytes to the output queue.
    fn collect(&mut self) {
        let bytes = self.enc.take_output();
        if self.sent == self.out.len() {
            self.out.clear();
            self.sent = 0;
        }
        self.out.extend_from_slice(&bytes);
    }

    /// Hands queued output to the writer. On error, what was not accepted
    /// stays queued.
    fn send(&mut self) -> io::Result<()> {
        let w = self.inner.as_mut().expect(ERR_STR);
        while self.sent < self.out.len() {
            match w.write(&self.out[self.sent..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to write compressed data")),
                Ok(n) => self.sent += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.out.clear();
        self.sent = 0;
        Ok(())
    }

    /// `Write::write`: queued output goes first (an error there consumes no
    /// input); then input is buffered and, when a chunk fills, compressed.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.send()?;
        if buf.is_empty() {
            return Ok(0);
        }
        if self.finished {
            return Err(io::Error::other("write after the stream was finished"));
        }
        let n = self.enc.push(buf);
        self.collect();
        // The input is taken; a write error now is reported by the next
        // call, which retries the queued output first.
        let _ = self.send();
        Ok(n)
    }

    fn sync_flush(&mut self) -> io::Result<()> {
        if !self.finished {
            self.enc.sync_flush();
            self.collect();
        }
        self.send()?;
        self.inner.as_mut().expect(ERR_STR).flush()
    }

    /// Ends the stream (once) with `trailer`, and hands everything to the
    /// writer.
    fn finish_with(&mut self, trailer: impl FnOnce() -> Vec<u8>) -> io::Result<()> {
        if !self.finished {
            self.enc.finish();
            self.collect();
            self.out.extend_from_slice(&trailer());
            self.finished = true;
        }
        self.send()
    }

    /// Replaces the writer and starts a new stream with `prefix` queued.
    fn reset(&mut self, writer: W, prefix: Vec<u8>) -> W {
        self.enc.reset();
        self.out = prefix;
        self.sent = 0;
        self.finished = false;
        self.inner.replace(writer).expect(ERR_STR)
    }
}

/// A raw DEFLATE encoder writing to `W`.
///
/// ```
/// use std::io::Write;
/// use verified_deflate::Compression;
/// use verified_deflate::write::DeflateEncoder;
///
/// let data = b"This is some test data";
/// let mut encoder = DeflateEncoder::new(Vec::new(), Compression::Default);
/// encoder.write_all(data)?;
/// let compressed_data = encoder.finish()?;
/// # assert_eq!(verified_deflate::decompress(&compressed_data).unwrap(), data);
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct DeflateEncoder<W: Write> {
    core: Core<W>,
}

impl<W: Write> DeflateEncoder<W> {
    /// Creates an encoder with the given options.
    pub fn new<O: Into<CompressionOptions>>(writer: W, options: O) -> DeflateEncoder<W> {
        DeflateEncoder { core: Core::new(writer, options.into(), Vec::new()) }
    }

    /// Finishes the stream and returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        self.core.finish_with(Vec::new)?;
        Ok(self.core.inner.take().expect(ERR_STR))
    }

    /// Finishes the current stream, then starts a new one (with the same
    /// options) on writer `w`, returning the old writer.
    pub fn reset(&mut self, w: W) -> io::Result<W> {
        self.core.finish_with(Vec::new)?;
        Ok(self.core.reset(w, Vec::new()))
    }
}

impl<W: Write> Write for DeflateEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.core.write(buf)
    }

    /// Sync flush: compresses all input so far and ends the output with an
    /// empty stored block, then flushes the writer.
    fn flush(&mut self) -> io::Result<()> {
        self.core.sync_flush()
    }
}

impl<W: Write> Drop for DeflateEncoder<W> {
    /// Finishes the stream, ignoring write errors (use `finish` to see them).
    fn drop(&mut self) {
        if self.core.inner.is_some() && !thread::panicking() {
            let _ = self.core.finish_with(Vec::new);
        }
    }
}

/// A zlib encoder writing to `W`: DEFLATE inside a zlib header and an
/// Adler-32 trailer.
///
/// ```
/// use std::io::Write;
/// use verified_deflate::Compression;
/// use verified_deflate::write::ZlibEncoder;
///
/// let data = b"This is some test data";
/// let mut encoder = ZlibEncoder::new(Vec::new(), Compression::Default);
/// encoder.write_all(data)?;
/// let compressed_data = encoder.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct ZlibEncoder<W: Write> {
    core: Core<W>,
    checksum: Adler32,
}

impl<W: Write> ZlibEncoder<W> {
    /// Creates an encoder with the given options.
    pub fn new<O: Into<CompressionOptions>>(writer: W, options: O) -> ZlibEncoder<W> {
        ZlibEncoder { core: Core::new(writer, options.into(), ZLIB_HEADER.to_vec()), checksum: Adler32::new() }
    }

    fn output_all(&mut self) -> io::Result<()> {
        let sum = self.checksum.finalize();
        self.core.finish_with(|| sum.to_be_bytes().to_vec())
    }

    /// Finishes the stream (with its checksum) and returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        self.output_all()?;
        Ok(self.core.inner.take().expect(ERR_STR))
    }

    /// Finishes the current stream, then starts a new one (with the same
    /// options) on `writer`, returning the old writer.
    pub fn reset(&mut self, writer: W) -> io::Result<W> {
        self.output_all()?;
        self.checksum = Adler32::new();
        Ok(self.core.reset(writer, ZLIB_HEADER.to_vec()))
    }

    /// The Adler-32 checksum of the data written so far.
    pub fn checksum(&self) -> u32 {
        self.checksum.finalize()
    }
}

impl<W: Write> Write for ZlibEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.core.write(buf)?;
        self.checksum.update(&buf[..n]);
        Ok(n)
    }

    /// Sync flush: compresses all input so far and ends the output with an
    /// empty stored block, then flushes the writer.
    fn flush(&mut self) -> io::Result<()> {
        self.core.sync_flush()
    }
}

impl<W: Write> Drop for ZlibEncoder<W> {
    /// Finishes the stream, ignoring write errors (use `finish` to see them).
    fn drop(&mut self) {
        if self.core.inner.is_some() && !thread::panicking() {
            let _ = self.output_all();
        }
    }
}

/// A gzip encoder writing to `W`: DEFLATE inside a gzip header and a
/// CRC-32 and length trailer.
///
/// ```
/// use std::io::Write;
/// use verified_deflate::Compression;
/// use verified_deflate::write::GzEncoder;
///
/// let data = b"This is some test data";
/// let mut encoder = GzEncoder::new(Vec::new(), Compression::Default);
/// encoder.write_all(data)?;
/// let compressed_data = encoder.finish()?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct GzEncoder<W: Write> {
    core: Core<W>,
    checksum: Crc32,
    amount: u32,
}

impl<W: Write> GzEncoder<W> {
    /// Creates an encoder with a header with default fields.
    pub fn new<O: Into<CompressionOptions>>(writer: W, options: O) -> GzEncoder<W> {
        GzEncoder::from_builder(GzBuilder::new(), writer, options)
    }

    /// Creates an encoder with the header described by `builder` (file
    /// name, comment and so on).
    pub fn from_builder<O: Into<CompressionOptions>>(builder: GzBuilder, writer: W, options: O) -> GzEncoder<W> {
        GzEncoder { core: Core::new(writer, options.into(), builder.into_header()), checksum: Crc32::new(), amount: 0 }
    }

    fn output_all(&mut self) -> io::Result<()> {
        let (crc, amount) = (self.checksum.finalize(), self.amount);
        self.core.finish_with(|| {
            let mut t = crc.to_le_bytes().to_vec();
            t.extend_from_slice(&amount.to_le_bytes());
            t
        })
    }

    /// Finishes the stream (with its trailer) and returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        self.output_all()?;
        Ok(self.core.inner.take().expect(ERR_STR))
    }

    fn reset_inner(&mut self, writer: W, builder: GzBuilder) -> io::Result<W> {
        self.output_all()?;
        self.checksum = Crc32::new();
        self.amount = 0;
        Ok(self.core.reset(writer, builder.into_header()))
    }

    /// Finishes the current stream, then starts a new one (with the same
    /// options and a header with default fields) on `writer`, returning the
    /// old writer.
    pub fn reset(&mut self, writer: W) -> io::Result<W> {
        self.reset_inner(writer, GzBuilder::new())
    }

    /// Like [`reset`](Self::reset), with the new stream's header described
    /// by `builder`.
    pub fn reset_with_builder(&mut self, writer: W, builder: GzBuilder) -> io::Result<W> {
        self.reset_inner(writer, builder)
    }

    /// The CRC-32 of the data written so far.
    pub fn checksum(&self) -> u32 {
        self.checksum.finalize()
    }
}

impl<W: Write> Write for GzEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.core.write(buf)?;
        self.checksum.update(&buf[..n]);
        self.amount = self.amount.wrapping_add(n as u32);
        Ok(n)
    }

    /// Sync flush: compresses all input so far and ends the output with an
    /// empty stored block, then flushes the writer.
    fn flush(&mut self) -> io::Result<()> {
        self.core.sync_flush()
    }
}

impl<W: Write> Drop for GzEncoder<W> {
    /// Finishes the stream, ignoring write errors (use `finish` to see them).
    fn drop(&mut self) {
        if self.core.inner.is_some() && !thread::panicking() {
            let _ = self.output_all();
        }
    }
}
