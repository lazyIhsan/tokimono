const LEFT_BITS: [u8; 4] = [0x40, 0x04, 0x02, 0x01]; // dot7, dot3, dot2, dot1 (bottom→top)
const RIGHT_BITS: [u8; 4] = [0x80, 0x20, 0x10, 0x08]; // dot8, dot6, dot5, dot4 (bottom→top)

fn quantize(value: f32, max: f32) -> u8 {
    if max <= 0.0 {
        return 0;
    }
    ((value / max) * 4.0).round().clamp(0.0, 4.0) as u8
}

fn column_mask(level: u8, bits: [u8; 4]) -> u8 {
    bits.iter().take(level as usize).fold(0u8, |acc, b| acc | b)
}

pub fn render(values: &[f32], max: f32) -> String {
    values
        .chunks(2)
        .map(|chunk| {
            let left = column_mask(quantize(chunk[0], max), LEFT_BITS);
            let right = chunk
                .get(1)
                .map(|&v| column_mask(quantize(v, max), RIGHT_BITS))
                .unwrap_or(0);
            char::from_u32(0x2800 | left as u32 | right as u32).unwrap()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_zero_renders_empty_cells() {
        assert_eq!(render(&[0.0, 0.0, 0.0, 0.0], 100.0), "\u{2800}\u{2800}");
    }

    #[test]
    fn all_max_renders_full_cells() {
        assert_eq!(
            render(&[100.0, 100.0, 100.0, 100.0], 100.0),
            "\u{28FF}\u{28FF}"
        );
    }

    #[test]
    fn odd_length_leaves_last_right_column_empty() {
        let out = render(&[100.0, 100.0, 100.0], 100.0);
        let chars: Vec<char> = out.chars().collect();
        assert_eq!(chars.len(), 2);
        assert_eq!(chars[0], '\u{28FF}');
        // last cell: full left column, empty right column
        assert_eq!(chars[1], '\u{2847}');
    }

    #[test]
    fn short_inputs_do_not_panic() {
        assert_eq!(render(&[], 100.0), "");
        let out = render(&[50.0], 100.0);
        let chars: Vec<char> = out.chars().collect();
        assert_eq!(chars.len(), 1);
        // 50% of 4 levels rounds to 2 -> dot7 + dot3 = 0x44
        assert_eq!(chars[0], '\u{2844}');
    }

    #[test]
    fn value_above_max_clamps_instead_of_panicking() {
        assert_eq!(render(&[150.0], 100.0), render(&[100.0], 100.0));
    }
}
