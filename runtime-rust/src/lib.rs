//! Pure, versioned primitives for an isolated memory-runtime experiment.
//!
//! This library does not read files, resolve paths, parse metadata JSON, or
//! perform any memory lifecycle operation. The default Python runtime does not
//! call it; the explicit maintainer launcher can opt into its fingerprint ABI.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum UTF-8 JSON envelope size, measured in bytes.
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
/// Maximum number of primitive requests in a single envelope.
pub const MAX_REQUESTS: usize = 128;
/// Wire protocol version.
pub const PROTOCOL: u32 = 1;
/// C ABI version, independent of the JSON protocol version.
pub const ABI_VERSION: u32 = 1;
/// Number of bytes in the raw fingerprint returned across the C ABI.
pub const FINGERPRINT_BYTES: usize = 32;

/// A refused primitive is valid protocol input and does not fail its batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Refusal {
    MissingOpening,
    MissingClosing,
    InvalidHex,
}

/// Raw slices of a delimited frontmatter record, without metadata decoding.
#[derive(Debug, Eq, PartialEq)]
pub struct Frontmatter<'a> {
    pub metadata_text: &'a str,
    pub body: &'a str,
}

/// Split exactly like `text[4:].split("\n---\n", 1)` after an opening check.
///
/// No newline, Unicode, metadata, body, or whitespace normalization occurs.
pub fn split_frontmatter(text: &str) -> Result<Frontmatter<'_>, Refusal> {
    let rest = text.strip_prefix("---\n").ok_or(Refusal::MissingOpening)?;
    let (metadata_text, body) = rest.split_once("\n---\n").ok_or(Refusal::MissingClosing)?;
    Ok(Frontmatter {
        metadata_text,
        body,
    })
}

/// SHA-256 with the canonical runtime's line-ending neutralization.
///
/// A NUL in the first 8192 bytes preserves every byte; otherwise only CRLF
/// pairs become LF. Lone CR bytes and NULs at later offsets are unchanged.
pub fn fingerprint_digest(bytes: &[u8]) -> [u8; FINGERPRINT_BYTES] {
    let mut digest = Sha256::new();
    if bytes[..bytes.len().min(8192)].contains(&0) {
        digest.update(bytes);
    } else {
        let mut start = 0;
        for (index, pair) in bytes.windows(2).enumerate() {
            if pair == b"\r\n" {
                digest.update(&bytes[start..index]);
                digest.update(b"\n");
                start = index + 2;
            }
        }
        digest.update(&bytes[start..]);
    }
    digest.finalize().into()
}

/// Lowercase hexadecimal form of [`fingerprint_digest`], used by the JSON CLI.
pub fn fingerprint_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in fingerprint_digest(bytes) {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 15) as usize] as char);
    }
    hex
}

/// Return the version of the exported C interface.
#[unsafe(no_mangle)]
pub extern "C" fn memory_kernel_abi_version() -> u32 {
    ABI_VERSION
}

/// Compute a line-ending-neutral fingerprint into caller-owned output memory.
///
/// Returns 0 after writing exactly 32 raw SHA-256 bytes, 1 for invalid arguments
/// detectable without dereferencing pointers, or 2 for a caught Rust panic. A
/// null `data` is accepted only with zero `len`. This function never returns
/// heap pointers, changes the process panic hook, or retains either buffer.
///
/// # Safety
///
/// If `len > 0`, `data` must point to `len` initialized readable bytes in one
/// live allocation; those bytes must not be mutated during the call. `output`
/// must point to at least 32 writable bytes in a live allocation and must not
/// overlap the input. Both allocations must remain valid for the call; their
/// addresses and extents must satisfy Rust's slice and pointer-copy rules.
/// `capacity` describes the caller's available output storage. Null pointers,
/// `capacity < 32`, and `len > isize::MAX` are rejected without dereferencing
/// either pointer. Other dangling, inaccessible, or overlapping pointers cannot
/// be validated here and violate the caller's safety obligations.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memory_kernel_fingerprint_v1(
    data: *const u8,
    len: usize,
    output: *mut u8,
    capacity: usize,
) -> i32 {
    if output.is_null()
        || capacity < FINGERPRINT_BYTES
        || (data.is_null() && len != 0)
        || len > isize::MAX as usize
    {
        return 1;
    }
    let result = std::panic::catch_unwind(|| {
        let bytes = if len == 0 {
            &[]
        } else {
            // SAFETY: The non-null pointer and bounded extent were checked
            // above; allocation validity and immutability are caller duties.
            unsafe { std::slice::from_raw_parts(data, len) }
        };
        let digest = fingerprint_digest(bytes);
        // SAFETY: The caller supplies a valid, nonoverlapping writable buffer;
        // the null/capacity checks above guarantee this copy's required size.
        unsafe { std::ptr::copy_nonoverlapping(digest.as_ptr(), output, FINGERPRINT_BYTES) };
    });
    if result.is_ok() { 0 } else { 2 }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, Refusal> {
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    if !value.len().is_multiple_of(2) {
        return Err(Refusal::InvalidHex);
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = nibble(pair[0]).ok_or(Refusal::InvalidHex)?;
            let low = nibble(pair[1]).ok_or(Refusal::InvalidHex)?;
            Ok((high << 4) | low)
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestEnvelope {
    protocol: u32,
    requests: Vec<Request>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
enum Request {
    #[serde(rename = "split-frontmatter-v1")]
    SplitFrontmatter { text: String },
    #[serde(rename = "fingerprint-bytes-v1")]
    FingerprintBytes { bytes_hex: String },
}

#[derive(Serialize)]
#[serde(tag = "status")]
enum PrimitiveResult<'a> {
    #[serde(rename = "ok")]
    Split {
        metadata_text: &'a str,
        body: &'a str,
    },
    #[serde(rename = "ok")]
    Fingerprint { sha256: String },
    #[serde(rename = "refused")]
    Refused { reason: Refusal },
}

#[derive(Serialize)]
struct ResponseEnvelope<'a> {
    protocol: u32,
    results: Vec<PrimitiveResult<'a>>,
}

/// A whole-envelope failure. Diagnostics deliberately contain no input data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    InvalidRequest,
    InputLimit,
    InvalidArguments,
    Io,
}

impl ProtocolError {
    /// Fixed UTF-8 JSON response, including a final newline.
    pub fn response(self) -> &'static [u8] {
        match self {
            Self::InvalidRequest => b"{\"protocol\":1,\"error\":\"invalid-request\"}\n",
            Self::InputLimit => b"{\"protocol\":1,\"error\":\"input-limit\"}\n",
            Self::InvalidArguments => b"{\"protocol\":1,\"error\":\"invalid-arguments\"}\n",
            Self::Io => b"{\"protocol\":1,\"error\":\"io\"}\n",
        }
    }
}

/// Validate one bounded strict JSON envelope and evaluate its primitive batch.
///
/// Unknown/duplicate fields, unsupported operations, malformed UTF-8/JSON,
/// excessive nesting, trailing JSON, and batches outside 1..=128 are errors.
/// Metadata inside a `text` string is deliberately not parsed as JSON.
pub fn evaluate(input: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(ProtocolError::InputLimit);
    }
    let envelope: RequestEnvelope =
        serde_json::from_slice(input).map_err(|_| ProtocolError::InvalidRequest)?;
    if envelope.protocol != PROTOCOL || !(1..=MAX_REQUESTS).contains(&envelope.requests.len()) {
        return Err(ProtocolError::InvalidRequest);
    }
    let results = envelope
        .requests
        .iter()
        .map(|request| match request {
            Request::SplitFrontmatter { text } => match split_frontmatter(text) {
                Ok(Frontmatter {
                    metadata_text,
                    body,
                }) => PrimitiveResult::Split {
                    metadata_text,
                    body,
                },
                Err(reason) => PrimitiveResult::Refused { reason },
            },
            Request::FingerprintBytes { bytes_hex } => match decode_hex(bytes_hex) {
                Ok(bytes) => PrimitiveResult::Fingerprint {
                    sha256: fingerprint_bytes(&bytes),
                },
                Err(reason) => PrimitiveResult::Refused { reason },
            },
        })
        .collect();
    let mut output = serde_json::to_vec(&ResponseEnvelope {
        protocol: PROTOCOL,
        results,
    })
    .map_err(|_| ProtocolError::Io)?;
    output.push(b'\n');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn batch(requests: serde_json::Value) -> serde_json::Value {
        let input = serde_json::to_vec(&json!({"protocol": 1, "requests": requests})).unwrap();
        serde_json::from_slice(&evaluate(&input).unwrap()).unwrap()
    }

    #[test]
    fn split_keeps_raw_metadata_and_body() {
        let text = "---\n{\"nan\": NaN, \"inf\": Infinity, \"escape\": \"\\ud800\"}\n---\n\r\nbody\r\n---\nagain\0e\u{301}";
        assert_eq!(
            split_frontmatter(text),
            Ok(Frontmatter {
                metadata_text: "{\"nan\": NaN, \"inf\": Infinity, \"escape\": \"\\ud800\"}",
                body: "\r\nbody\r\n---\nagain\0e\u{301}",
            })
        );
        assert_eq!(
            split_frontmatter("---\n\n---\n"),
            Ok(Frontmatter {
                metadata_text: "",
                body: ""
            })
        );
    }

    #[test]
    fn split_refuses_only_delimiter_failures() {
        for text in [
            "",
            "---\r\n{}\n---\n",
            "\u{feff}---\n{}\n---\n",
            " ---\n{}\n---\n",
        ] {
            assert_eq!(split_frontmatter(text), Err(Refusal::MissingOpening));
        }
        for text in [
            "---\n{}",
            "---\n---\n",
            "---\n---\nbody",
            "---\n{}\n---",
            "---\n{}\r\n---\r\nbody",
        ] {
            assert_eq!(split_frontmatter(text), Err(Refusal::MissingClosing));
        }
    }

    #[test]
    fn fingerprint_matches_known_sha256_vectors() {
        assert_eq!(
            fingerprint_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            fingerprint_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            fingerprint_bytes(b"a\r\nb\r\n\r\n"),
            fingerprint_bytes(b"a\nb\n\n")
        );
        assert_ne!(fingerprint_bytes(b"a\rb"), fingerprint_bytes(b"a\nb"));
        assert_ne!(fingerprint_bytes(b"a\0\r\n"), fingerprint_bytes(b"a\0\n"));
        // Python bytes.replace is one pass: CRCRLF becomes CRLF, not LF.
        assert_eq!(
            fingerprint_bytes(b"a\r\r\nb"),
            "18745f36a05e29072709042d6062ce54f1b08ff36c27ba80c39f81fb010c8ce2"
        );
    }

    #[test]
    fn fingerprint_nul_cutoff_is_exactly_8192_bytes() {
        let mut inside = vec![b'x'; 8191];
        inside.extend_from_slice(b"\0\r\n");
        let mut changed = inside.clone();
        changed.remove(changed.len() - 2);
        assert_ne!(fingerprint_bytes(&inside), fingerprint_bytes(&changed));
        let mut outside = vec![b'x'; 8192];
        outside.extend_from_slice(b"\0\r\n");
        let mut changed = outside.clone();
        changed.remove(changed.len() - 2);
        assert_eq!(fingerprint_bytes(&outside), fingerprint_bytes(&changed));
    }

    fn ffi_digest(bytes: &[u8]) -> [u8; FINGERPRINT_BYTES] {
        // Guard bytes establish that the ABI writes exactly 32 bytes even
        // when the caller provides more capacity.
        let mut output = [0xa5; FINGERPRINT_BYTES + 8];
        // SAFETY: Both buffers are live, disjoint allocations of the given
        // extents; the input remains immutable for the duration of the call.
        let result = unsafe {
            memory_kernel_fingerprint_v1(
                bytes.as_ptr(),
                bytes.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        assert_eq!(result, 0);
        assert_eq!(&output[FINGERPRINT_BYTES..], &[0xa5; 8]);
        output[..FINGERPRINT_BYTES].try_into().unwrap()
    }

    #[test]
    fn ffi_version_and_known_digest_vectors() {
        assert_eq!(memory_kernel_abi_version(), 1);
        assert_eq!(
            ffi_digest(b"abc").to_vec(),
            decode_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").unwrap()
        );
        let large = vec![b'a'; 1_000_000];
        assert_eq!(
            ffi_digest(&large).to_vec(),
            decode_hex("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0").unwrap()
        );
    }

    #[test]
    fn ffi_accepts_null_input_for_empty_bytes() {
        let mut output = [0; FINGERPRINT_BYTES];
        // SAFETY: Zero-length null input is explicitly accepted by the ABI,
        // and output is a live writable 32-byte allocation.
        let result = unsafe {
            memory_kernel_fingerprint_v1(std::ptr::null(), 0, output.as_mut_ptr(), output.len())
        };
        assert_eq!(result, 0);
        assert_eq!(
            output.to_vec(),
            decode_hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").unwrap()
        );
    }

    #[test]
    fn ffi_rejects_invalid_arguments_without_writing_output() {
        let input = b"abc";
        let mut output = [0xa5; FINGERPRINT_BYTES];
        // SAFETY: Each invalid argument below is one explicitly rejected
        // before any dereference; all remaining buffers are live and disjoint.
        unsafe {
            assert_eq!(
                memory_kernel_fingerprint_v1(
                    input.as_ptr(),
                    input.len(),
                    std::ptr::null_mut(),
                    FINGERPRINT_BYTES
                ),
                1
            );
            assert_eq!(
                memory_kernel_fingerprint_v1(
                    input.as_ptr(),
                    input.len(),
                    output.as_mut_ptr(),
                    FINGERPRINT_BYTES - 1
                ),
                1
            );
            assert_eq!(
                memory_kernel_fingerprint_v1(
                    std::ptr::null(),
                    1,
                    output.as_mut_ptr(),
                    output.len()
                ),
                1
            );
            assert_eq!(
                memory_kernel_fingerprint_v1(
                    input.as_ptr(),
                    isize::MAX as usize + 1,
                    output.as_mut_ptr(),
                    output.len()
                ),
                1
            );
        }
        assert_eq!(output, [0xa5; FINGERPRINT_BYTES]);
    }

    #[test]
    fn ffi_preserves_crlf_and_nul_boundary_semantics() {
        for bytes in [b"a\r\nb\r".as_slice(), b"a\0\r\nb", b"a\r\r\nb"] {
            assert_eq!(ffi_digest(bytes), fingerprint_digest(bytes));
        }
        assert_eq!(ffi_digest(b"a\r\nb"), fingerprint_digest(b"a\nb"));
        assert_ne!(ffi_digest(b"a\0\r\nb"), fingerprint_digest(b"a\0\nb"));
        let mut input = vec![b'x'; 8191];
        input.extend_from_slice(b"\0\r\n");
        assert_eq!(ffi_digest(&input), fingerprint_digest(&input));
        let mut input = vec![b'x'; 8192];
        input.extend_from_slice(b"\0\r\n");
        let mut normalized = vec![b'x'; 8192];
        normalized.extend_from_slice(b"\0\n");
        assert_eq!(ffi_digest(&input), fingerprint_digest(&normalized));
    }

    #[test]
    fn batch_order_and_refusals_are_preserved() {
        let result = batch(json!([
            {"operation":"split-frontmatter-v1","text":"bad"},
            {"operation":"fingerprint-bytes-v1","bytes_hex":"616263"},
            {"operation":"fingerprint-bytes-v1","bytes_hex":"0g"},
            {"operation":"split-frontmatter-v1","text":"---\n{}\n---\nbody"}
        ]));
        assert_eq!(result["protocol"], 1);
        assert_eq!(
            result["results"][0],
            json!({"status":"refused","reason":"missing-opening"})
        );
        assert_eq!(result["results"][1]["sha256"], fingerprint_bytes(b"abc"));
        assert_eq!(
            result["results"][2],
            json!({"status":"refused","reason":"invalid-hex"})
        );
        assert_eq!(
            result["results"][3],
            json!({"status":"ok","metadata_text":"{}","body":"body"})
        );
    }

    #[test]
    fn hex_is_ascii_pairs_without_whitespace_or_prefixes() {
        assert_eq!(decode_hex("aB00Ff"), Ok(vec![0xab, 0, 255]));
        assert_eq!(decode_hex(""), Ok(vec![]));
        for value in ["0", "000", "0x12", "aa bb", "é", "１２", "\n00", "gg"] {
            assert_eq!(decode_hex(value), Err(Refusal::InvalidHex));
        }
    }

    #[test]
    fn strict_envelope_and_operation_fields() {
        for input in [
            r#"{}"#,
            r#"{"protocol":1,"requests":[]}"#,
            r#"{"protocol":2,"requests":[{"operation":"fingerprint-bytes-v1","bytes_hex":""}]}"#,
            r#"{"protocol":1,"protocol":1,"requests":[]}"#,
            r#"{"protocol":1,"requests":[],"requests":[]}"#,
            r#"{"protocol":1,"requests":[{"operation":"fingerprint-bytes-v1","bytes_hex":""}],"extra":0}"#,
            r#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","text":"x","extra":0}]}"#,
            r#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","text":"x","text":"y"}]}"#,
            r#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","operation":"split-frontmatter-v1","text":"x"}]}"#,
            r#"{"protocol":1,"requests":[{"operation":"other","text":"x"}]}"#,
            r#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","text":null}]}"#,
            r#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","text":"\ud800"}]}"#,
            r#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","text":"x"}]}{}"#,
            r#"{"protocol":1.0,"requests":[{"operation":"fingerprint-bytes-v1","bytes_hex":""}]}"#,
        ] {
            assert_eq!(
                evaluate(input.as_bytes()),
                Err(ProtocolError::InvalidRequest),
                "{input}"
            );
        }
        assert_eq!(evaluate(b"\xff"), Err(ProtocolError::InvalidRequest));
    }

    #[test]
    fn rejects_excessive_nesting_without_panicking() {
        let input = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        assert_eq!(
            evaluate(input.as_bytes()),
            Err(ProtocolError::InvalidRequest)
        );
        // Exercise nested content buffered by the internally tagged request,
        // rather than rejecting a root array before traversing its contents.
        let input = format!(
            "{{\"protocol\":1,\"requests\":[{{\"operation\":\"split-frontmatter-v1\",\"text\":{}0{}}}]}}",
            "[".repeat(256),
            "]".repeat(256)
        );
        assert_eq!(
            evaluate(input.as_bytes()),
            Err(ProtocolError::InvalidRequest)
        );
    }

    #[test]
    fn request_count_and_input_size_are_bounded() {
        let item = json!({"operation":"fingerprint-bytes-v1","bytes_hex":""});
        assert_eq!(
            batch(json!(vec![item.clone(); MAX_REQUESTS]))["results"]
                .as_array()
                .unwrap()
                .len(),
            MAX_REQUESTS
        );
        let input =
            serde_json::to_vec(&json!({"protocol":1,"requests":vec![item; MAX_REQUESTS + 1]}))
                .unwrap();
        assert_eq!(evaluate(&input), Err(ProtocolError::InvalidRequest));
        assert_eq!(
            evaluate(&vec![b' '; MAX_INPUT_BYTES + 1]),
            Err(ProtocolError::InputLimit)
        );
        let mut input =
            br#"{"protocol":1,"requests":[{"operation":"fingerprint-bytes-v1","bytes_hex":""}]}"#
                .to_vec();
        input.resize(MAX_INPUT_BYTES, b' ');
        assert!(evaluate(&input).is_ok());
    }
}
