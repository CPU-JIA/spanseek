/// Decompresses one PalmDOC text record.
///
/// The format is a byte-oriented LZ77 stream: 0x01..=0x08 copy that many
/// literals, 0x09..=0x7f are literals, 0x80..=0xbf introduce a two-byte
/// distance/length pair, and 0xc0..=0xff encode a space plus a XORed character.
/// A corrupt back-reference stops the record rather than panicking.
pub fn decompress(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len().saturating_mul(2));
    let mut pos = 0usize;

    while pos < data.len() {
        let byte = data[pos];
        pos += 1;
        match byte {
            0 => output.push(0),
            1..=8 => {
                let count = byte as usize;
                let end = pos.saturating_add(count).min(data.len());
                output.extend_from_slice(&data[pos..end]);
                pos = end;
            }
            0x09..=0x7f => output.push(byte),
            0x80..=0xbf => {
                if pos >= data.len() {
                    break;
                }
                let pair = (u16::from(byte) << 8) | u16::from(data[pos]);
                pos += 1;
                let distance = ((pair >> 3) & 0x07ff) as usize;
                let length = ((pair & 0x0007) as usize) + 3;
                if distance == 0 || distance > output.len() {
                    break;
                }
                for _ in 0..length {
                    let next = output[output.len() - distance];
                    output.push(next);
                }
            }
            _ => {
                output.push(b' ');
                output.push(byte ^ 0x80);
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::decompress;

    #[test]
    fn literals_and_lz_overlap_are_decoded() {
        // "abc" then a distance-3, length-5 reference produces "abcabcab".
        assert_eq!(decompress(&[b'a', b'b', b'c', 0x80, 0x1a]), b"abcabcab");
    }

    #[test]
    fn space_xor_literals_and_short_literal_runs_are_supported() {
        assert_eq!(decompress(&[3, b'a', b'b', b'c', 0xc1]), b"abc A");
    }

    #[test]
    fn invalid_back_reference_stops_without_panicking() {
        assert_eq!(decompress(&[b'a', 0x80, 0x05]), b"a");
    }
}
