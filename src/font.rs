//! See docs/DESIGN-NOTES.md.

const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
];

pub const GLYPH_W: u32 = 5;
pub const GLYPH_H: u32 = 7;

pub fn digit_rows(d: u8) -> [u8; 7] {
    DIGITS[(d % 10) as usize]
}

pub fn digit_pixel(d: u8, col: u32, row: u32) -> bool {
    if col >= GLYPH_W || row >= GLYPH_H {
        return false;
    }
    let bits = digit_rows(d)[row as usize];
    (bits >> (GLYPH_W - 1 - col)) & 1 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_one_has_a_stem() {
        assert!(digit_pixel(1, 2, 0));
        assert!(digit_pixel(1, 2, 6));
        assert!(!digit_pixel(1, 0, 0));
    }

    #[test]
    fn out_of_range_is_blank() {
        assert!(!digit_pixel(8, GLYPH_W, 0));
        assert!(!digit_pixel(8, 0, GLYPH_H));
    }

    #[test]
    fn wraps_modulo_ten() {
        assert_eq!(digit_rows(0), digit_rows(10));
    }
}
