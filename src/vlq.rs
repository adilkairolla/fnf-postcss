//! Base64 VLQ codec used by the `mappings` field of a source map, plus the
//! plain base64 codec used for inline maps.
//!
//! Replaces the `source-map-js` and `Buffer` dependencies of the JS version.

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const VLQ_BASE_SHIFT: i64 = 5;
const VLQ_BASE: i64 = 1 << VLQ_BASE_SHIFT;
const VLQ_BASE_MASK: i64 = VLQ_BASE - 1;
const VLQ_CONTINUATION_BIT: i64 = VLQ_BASE;

fn base64_decode_char(byte: u8) -> Option<i64> {
    let value = match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(value as i64)
}

/// Appends the base64 VLQ encoding of a signed integer.
pub fn encode_vlq(value: i64, out: &mut String) {
    let mut vlq = if value < 0 {
        ((-value) << 1) | 1
    } else {
        value << 1
    };

    loop {
        let mut digit = vlq & VLQ_BASE_MASK;
        vlq >>= VLQ_BASE_SHIFT;
        if vlq > 0 {
            digit |= VLQ_CONTINUATION_BIT;
        }
        out.push(BASE64_CHARS[digit as usize] as char);
        if vlq == 0 {
            break;
        }
    }
}

/// Decodes one base64 VLQ value, returning it with the number of bytes read.
pub fn decode_vlq(bytes: &[u8]) -> Option<(i64, usize)> {
    let mut result: i64 = 0;
    let mut shift = 0;
    let mut read = 0;

    loop {
        let byte = *bytes.get(read)?;
        let digit = base64_decode_char(byte)?;
        read += 1;

        let has_continuation = (digit & VLQ_CONTINUATION_BIT) != 0;
        result += (digit & VLQ_BASE_MASK) << shift;
        shift += VLQ_BASE_SHIFT;

        if !has_continuation {
            break;
        }
    }

    let negative = (result & 1) == 1;
    result >>= 1;
    Some((if negative { -result } else { result }, read))
}

/// Standard base64 encoding, for `data:application/json;base64,` maps.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(BASE64_CHARS[(triple >> 18 & 0x3f) as usize] as char);
        out.push(BASE64_CHARS[(triple >> 12 & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64_CHARS[(triple >> 6 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_CHARS[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Standard base64 decoding. Whitespace is ignored; returns `None` on any
/// other invalid input.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut buffer: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::with_capacity(text.len() / 4 * 3);

    for byte in text.bytes() {
        match byte {
            b'=' => break,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => {}
        }
        let value = base64_decode_char(byte)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_vlq() {
        for value in [-100000, -257, -256, -16, -1, 0, 1, 15, 16, 255, 256, 123456] {
            let mut encoded = String::new();
            encode_vlq(value, &mut encoded);
            let (decoded, read) = decode_vlq(encoded.as_bytes()).unwrap();
            assert_eq!(decoded, value, "value {value} encoded as {encoded}");
            assert_eq!(read, encoded.len());
        }
    }

    #[test]
    fn matches_known_vlq_encodings() {
        let mut out = String::new();
        encode_vlq(0, &mut out);
        assert_eq!(out, "A");
        out.clear();
        encode_vlq(1, &mut out);
        assert_eq!(out, "C");
        out.clear();
        encode_vlq(-1, &mut out);
        assert_eq!(out, "D");
        out.clear();
        encode_vlq(16, &mut out);
        assert_eq!(out, "gB");
    }

    #[test]
    fn round_trips_base64() {
        for text in ["", "a", "ab", "abc", "abcd", "{\"version\":3}"] {
            let encoded = base64_encode(text.as_bytes());
            let decoded = base64_decode(&encoded).unwrap();
            assert_eq!(String::from_utf8(decoded).unwrap(), text);
        }
        assert_eq!(base64_encode(b"sure"), "c3VyZQ==");
        assert_eq!(base64_encode(b"sure."), "c3VyZS4=");
    }
}
