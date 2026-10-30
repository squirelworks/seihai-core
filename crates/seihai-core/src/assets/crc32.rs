//! CRC-32 (ISO 3309 / ITU-T V.42) as used by the Graal client PNG writers.
//! Polynomial 0xEDB88320, initial value 0xFFFFFFFF, final XOR 0xFFFFFFFF.

const POLY: u32 = 0xEDB88320;

/// Precomputed CRC-32 table (identical algorithm to the client’s FUN_* table builder).
static CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLY;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// The raw CRC-32 update: run `data` through the register `reg` with no
/// initial value and no final inversion. `crc32(d)` is
/// `!crc32_update(0xFFFFFFFF, d)`; chaining calls (as the tamper check
/// does) feeds one call's register into the next.
pub fn crc32_update(mut reg: u32, data: &[u8]) -> u32 {
    for &b in data {
        let idx = ((reg ^ b as u32) & 0xFF) as usize;
        reg = CRC_TABLE[idx] ^ (reg >> 8);
    }
    reg
}

/// Compute CRC-32 over `data`.
/// Matches the client’s update function (initial 0xFFFFFFFF, final XOR 0xFFFFFFFF).
pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0xFFFFFFFF, data) ^ 0xFFFFFFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn update_is_the_raw_register_step() {
        assert_eq!(crc32_update(0xFFFFFFFF, b"123456789") ^ 0xFFFFFFFF, 0xCBF43926);
        // Splitting the input and feeding the register on gives the same result.
        let r = crc32_update(crc32_update(0xFFFFFFFF, b"1234"), b"56789");
        assert_eq!(r ^ 0xFFFFFFFF, 0xCBF43926);
    }

    #[test]
    fn known_vector() {
        // "123456789" is the standard CRC-32 check value
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }
}
