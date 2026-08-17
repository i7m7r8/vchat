/// Simple RFC4648 base32 encoder (no padding)
pub fn encode_base32(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = String::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in data {
        buf = (buf << 8) | byte as u32;
        bits += 8;

        while bits >= 5 {
            bits -= 5;
            result.push(ALPHABET[((buf >> bits) & 0x1F) as usize] as char);
        }
    }

    if bits > 0 {
        result.push(ALPHABET[((buf << (5 - bits)) & 0x1F) as usize] as char);
    }

    result
}
