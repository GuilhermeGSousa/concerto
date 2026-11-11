const CHARS: &[u8] = b" .:-=+*#%@";

pub fn luma_to_char(luma: u8) -> char {
    let idx = (luma as usize * (CHARS.len() - 1)) / 255;
    CHARS[idx] as char
}

pub fn pixels_to_ascii_into(
    data: &[u8],
    width: u32,
    height: u32,
    padded_bpr: u32,
    out: &mut String,
) {
    out.clear();
    let needed = (width as usize + 2) * height as usize;
    if out.capacity() < needed {
        out.reserve(needed - out.capacity());
    }

    for row in 0..height {
        for col in 0..width {
            let offset = (row * padded_bpr + col * 4) as usize;
            let r = data[offset] as f32;
            let g = data[offset + 1] as f32;
            let b = data[offset + 2] as f32;
            let luma = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
            out.push(luma_to_char(luma));
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Shade {
        luma: u8,
        char: char,
    }

    #[test]
    fn luma_walks_the_ramp_from_a_space_for_black_to_an_at_for_white() {
        let cases = [
            Shade { luma: 0, char: ' ' },
            Shade {
                luma: 28,
                char: ' ',
            },
            Shade {
                luma: 29,
                char: '.',
            },
            Shade {
                luma: 128,
                char: '=',
            },
            Shade {
                luma: 254,
                char: '%',
            },
            Shade {
                luma: 255,
                char: '@',
            },
        ];
        for case in &cases {
            assert_eq!(luma_to_char(case.luma), case.char, "{}", case.luma);
        }
        let ramp: Vec<usize> = (0..=255u8)
            .map(|luma| {
                CHARS
                    .iter()
                    .position(|&c| c as char == luma_to_char(luma))
                    .unwrap()
            })
            .collect();
        assert!(ramp.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}
