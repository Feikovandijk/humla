//! Splitting a transcript too long for a local model's context into pieces
//! that each fit, for the summary's map-reduce path (8 GB Macs).

/// Smallest piece worth a model call, so a budget eaten by huge typed notes
/// still makes progress instead of producing empty pieces.
pub(super) const MIN_PIECE_CHARS: usize = 4000;

/// Characters left for transcript text once `fixed_chars` of prompt are spent.
pub(super) fn piece_budget(budget_chars: usize, fixed_chars: usize) -> usize {
    budget_chars.saturating_sub(fixed_chars).max(MIN_PIECE_CHARS)
}

/// Packs whole lines (speaker turns) into pieces of at most `max_chars` bytes,
/// in order. A line longer than that is cut at whitespace, or at a char
/// boundary when it has none. Blank lines are dropped; no words are.
pub(super) fn split_transcript(text: &str, max_chars: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        for seg in cut_line(line, max_chars) {
            if seg.trim().is_empty() {
                continue;
            }
            if cur.is_empty() {
                cur.push_str(seg);
            } else if cur.len() + 1 + seg.len() <= max_chars {
                cur.push('\n');
                cur.push_str(seg);
            } else {
                pieces.push(std::mem::replace(&mut cur, seg.to_string()));
            }
        }
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
}

fn cut_line(mut line: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    while line.len() > max {
        let mut b = (0..=max).rev().find(|&i| line.is_char_boundary(i)).unwrap_or(0);
        if b == 0 {
            // max is smaller than one char: take that char whole.
            b = line.chars().next().map_or(line.len(), char::len_utf8);
        }
        match line[..b].rfind(char::is_whitespace).filter(|&i| i > 0) {
            Some(i) => {
                out.push(&line[..i]);
                line = line[i..].trim_start();
            }
            None => {
                out.push(&line[..b]);
                line = &line[b..];
            }
        }
    }
    out.push(line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<&str> {
        s.split_whitespace().collect()
    }

    #[test]
    fn pieces_fit_and_keep_every_word_in_order() {
        let text = (0..200)
            .map(|i| format!("Speaker {}: line number {i}", i % 3))
            .collect::<Vec<_>>()
            .join("\n");
        let pieces = split_transcript(&text, 500);
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|p| p.len() <= 500));
        assert_eq!(words(&pieces.join("\n")), words(&text));
    }

    #[test]
    fn one_huge_line_is_split_at_whitespace() {
        let text = "ord ".repeat(1000);
        let pieces = split_transcript(text.trim(), 300);
        assert!(pieces.iter().all(|p| p.len() <= 300 && !p.starts_with(' ')));
        assert_eq!(words(&pieces.join(" ")), words(&text));
    }

    #[test]
    fn multibyte_text_never_splits_inside_a_char() {
        let text = "æøå".repeat(500); // no whitespace at all
        let pieces = split_transcript(&text, 301);
        assert_eq!(pieces.concat(), text);
    }

    #[test]
    fn text_that_fits_is_one_piece() {
        assert_eq!(split_transcript("a\nb", 100), vec!["a\nb".to_string()]);
    }

    #[test]
    fn piece_budget_has_a_floor() {
        assert_eq!(piece_budget(10_000, 4_000), 6_000);
        assert_eq!(piece_budget(10_000, 50_000), MIN_PIECE_CHARS);
    }
}
