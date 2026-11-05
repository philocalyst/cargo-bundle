//! SquashFS decompression without C.
//!
//! backhand's own `xz` and `zstd` features link `liblzma` and `zstd-sys`.
//! Instead, [`Pure`] is plugged in as backhand's [`CompressionAction`]: xz goes
//! through [`lzma_rs`], zstd through [`ruzstd`], and everything else (gzip, lzo,
//! lz4, uncompressed) to backhand's own pure-Rust [`DefaultCompressor`].

use backhand::compression::{CompressionAction, Compressor, DefaultCompressor};
use backhand::kind::Kind;
use backhand::{BackhandError, BufReadSeek, FilesystemCompressor, FilesystemReader, SuperBlock};

/// Open the SquashFS that starts `offset` bytes into `reader`.
pub(crate) fn open<R: BufReadSeek + 'static>(
    reader: R,
    offset: u64,
) -> Result<FilesystemReader<'static>, BackhandError> {
    FilesystemReader::from_reader_with_offset_and_kind(reader, offset, Kind::new_v4(&Pure))
}

struct Pure;

impl CompressionAction for Pure {
    type Error = BackhandError;
    type Compressor = Compressor;
    type FilesystemCompressor = FilesystemCompressor;
    type SuperBlock = SuperBlock;

    fn decompress(
        &self,
        bytes: &[u8],
        out: &mut Vec<u8>,
        compressor: Compressor,
    ) -> Result<(), BackhandError> {
        match compressor {
            #[cfg(feature = "zstd")]
            Compressor::Zstd => {
                use std::io::Read;
                ruzstd::decoding::StreamingDecoder::new(bytes)
                    .map_err(|error| codec_error("zstd", error))?
                    .read_to_end(out)
                    .map_err(BackhandError::StdIo)?;
                Ok(())
            }
            #[cfg(feature = "xz")]
            Compressor::Xz => lzma_rs::xz_decompress(&mut { bytes }, out)
                .map_err(|error| codec_error("xz", error)),
            // With `xz` / `zstd` disabled they land here and come back as
            // backhand's "unsupported compression" error.
            other => DefaultCompressor.decompress(bytes, out, other),
        }
    }

    // This crate only decodes through `Pure`; packing builds its own
    // `FilesystemCompressor`. The trait still wants the encoder half.
    fn compress(
        &self,
        bytes: &[u8],
        fc: FilesystemCompressor,
        block_size: u32,
    ) -> Result<Vec<u8>, BackhandError> {
        DefaultCompressor.compress(bytes, fc, block_size)
    }

    fn compression_options(
        &self,
        superblock: &mut SuperBlock,
        kind: &Kind,
        fc: FilesystemCompressor,
    ) -> Result<Option<Vec<u8>>, BackhandError> {
        DefaultCompressor.compression_options(superblock, kind, fc)
    }
}

#[cfg(any(feature = "xz", feature = "zstd"))]
fn codec_error(codec: &str, error: impl std::fmt::Display) -> BackhandError {
    BackhandError::CompressionInit(format!("{codec}: {error}"))
}
