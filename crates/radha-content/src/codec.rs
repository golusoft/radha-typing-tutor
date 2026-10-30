//! Compression and hashing for stored content (P04 storage).
//!
//! - **Text column**: each passage is stored deflate-compressed
//!   ([`compress_text`] / [`decompress_text`]); the codec name is stored next
//!   to it so rows written before compression (`plain`) stay readable.
//! - **Bundle files**: a whole product bundle is zlib-compressed JSON
//!   ([`pack`] / [`unpack`]); zlib's Adler-32 catches a truncated file.
//! - **Integrity**: [`sha256_hex`] of the UTF-8 text is stored per passage
//!   and re-checked before a passage reaches the database.
//!
//! Decompression is bounded ([`MAX_TEXT_BYTES`], [`MAX_BUNDLE_BYTES`]) so a
//! corrupt or hostile file cannot exhaust memory.

use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};
use miniz_oxide::inflate::{decompress_to_vec_with_limit, decompress_to_vec_zlib_with_limit};
use radha_core::{CoreError, Result};
use sha2::{Digest, Sha256};

/// Codec name of a deflate-compressed text column.
pub const CODEC_DEFLATE: &str = "deflate";
/// Codec name of an uncompressed text column (rows from before P04).
pub const CODEC_PLAIN: &str = "plain";

/// Largest decoded passage accepted (the longest shipped passage is far
/// below this).
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Largest decoded bundle accepted.
pub const MAX_BUNDLE_BYTES: usize = 64 * 1024 * 1024;

/// Compression level (miniz scale 0–10): the best ratio; compression only
/// runs at import time.
const LEVEL: u8 = 10;

/// Lowercase hex sha256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Deflate one passage for the text column.
#[must_use]
pub fn compress_text(text: &str) -> Vec<u8> {
    compress_to_vec(text.as_bytes(), LEVEL)
}

/// Decode a text column written with `codec`.
///
/// # Errors
///
/// [`CoreError::Malformed`] for an unknown codec, corrupt deflate data, a
/// passage over [`MAX_TEXT_BYTES`] or invalid UTF-8.
pub fn decompress_text(codec: &str, body: &[u8]) -> Result<String> {
    let bytes = match codec {
        CODEC_DEFLATE => decompress_to_vec_with_limit(body, MAX_TEXT_BYTES)
            .map_err(|e| bad("text column", format!("deflate: {e:?}")))?,
        CODEC_PLAIN => body.to_vec(),
        other => return Err(bad("text column", format!("unknown codec {other:?}"))),
    };
    String::from_utf8(bytes).map_err(|e| bad("text column", e.to_string()))
}

/// Compress a serialised bundle for shipping.
#[must_use]
pub fn pack(json: &str) -> Vec<u8> {
    compress_to_vec_zlib(json.as_bytes(), LEVEL)
}

/// Decode a shipped bundle file back to JSON.
///
/// # Errors
///
/// [`CoreError::Malformed`] for corrupt zlib data (including a checksum
/// mismatch), an oversized bundle or invalid UTF-8.
pub fn unpack(bytes: &[u8]) -> Result<String> {
    let json = decompress_to_vec_zlib_with_limit(bytes, MAX_BUNDLE_BYTES)
        .map_err(|e| bad("content bundle", format!("zlib: {e:?}")))?;
    String::from_utf8(json).map_err(|e| bad("content bundle", e.to_string()))
}

fn bad(kind: &'static str, detail: String) -> CoreError {
    CoreError::malformed(kind, "<compressed>", detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_through_deflate_and_shrinks() {
        let text = "भारत एक विशाल देश है। ".repeat(200);
        let z = compress_text(&text);
        assert!(z.len() * 10 < text.len(), "{} -> {}", text.len(), z.len());
        assert_eq!(decompress_text(CODEC_DEFLATE, &z).unwrap(), text);
        assert_eq!(decompress_text(CODEC_PLAIN, text.as_bytes()).unwrap(), text);
    }

    #[test]
    fn corrupt_or_unknown_columns_are_errors_not_panics() {
        assert!(decompress_text(CODEC_DEFLATE, b"\xff\xff\xff\xff").is_err());
        assert!(decompress_text("zstd", b"abc").is_err());
        assert!(
            decompress_text(CODEC_PLAIN, b"\xff").is_err(),
            "invalid UTF-8"
        );
    }

    #[test]
    fn bundles_round_trip_and_a_truncated_file_is_rejected() {
        let json = r#"{"format":2,"product":"x"}"#.repeat(50);
        let packed = pack(&json);
        assert_eq!(unpack(&packed).unwrap(), json);
        assert!(unpack(&packed[..packed.len() - 3]).is_err());
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
