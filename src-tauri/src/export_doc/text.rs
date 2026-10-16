//! Blocks → a plain-text document. No markup survives: emphasis is dropped,
//! lists get `•` bullets with hanging indents, tables are padded into columns.
//! Paragraphs are not hard-wrapped, so the reader's editor wraps them to fit.

use super::{spans_text, Block};

pub fn render_text(blocks: &[Block]) -> String {
    join(blocks, "\n\n", 0)
}

fn join(blocks: &[Block], sep: &str, depth: usize) -> String {
    blocks.iter().map(|b| block(b, depth)).collect::<Vec<_>>().join(sep)
}

fn block(b: &Block, depth: usize) -> String {
    match b {
        Block::Heading { text, .. } | Block::Paragraph(text) => spans_text(text),
        Block::List { start, items } => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let marker = match start {
                    Some(n) => format!("{}. ", n + i as u64),
                    None => format!("{} ", if depth == 0 { '•' } else { '◦' }),
                };
                hang(&marker, &join(item, "\n", depth + 1))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Quote(inner) => prefix_lines(&join(inner, "\n\n", depth), "> "),
        Block::Code(code) => prefix_lines(code, "    "),
        Block::Rule => "* * *".to_string(),
        Block::Table { header, rows } => {
            let cell = |c: &Vec<super::Span>| spans_text(c).replace('\n', " ");
            let mut grid: Vec<Vec<String>> = vec![header.iter().map(cell).collect()];
            grid.extend(rows.iter().map(|r| r.iter().map(cell).collect()));
            table(&grid)
        }
    }
}

/// Put `marker` before the first line and indent the rest to line up under it.
fn hang(marker: &str, body: &str) -> String {
    let pad = " ".repeat(marker.chars().count());
    body.lines()
        .enumerate()
        .map(|(i, line)| match (i, line.is_empty()) {
            (0, _) => format!("{marker}{line}"),
            (_, true) => String::new(),
            _ => format!("{pad}{line}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn prefix_lines(body: &str, prefix: &str) -> String {
    body.lines()
        .map(|line| if line.is_empty() { prefix.trim_end().to_string() } else { format!("{prefix}{line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Columns padded to their widest cell, the header ruled off with dashes.
fn table(grid: &[Vec<String>]) -> String {
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| grid.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0))
        .collect();
    let row = |cells: &[String]| {
        widths
            .iter()
            .enumerate()
            .map(|(c, w)| {
                let s = cells.get(c).map(String::as_str).unwrap_or("");
                format!("{s}{}", " ".repeat(w - s.chars().count()))
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut lines = vec![row(&grid[0])];
    lines.push(widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  "));
    lines.extend(grid[1..].iter().map(|r| row(r)));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::parse_markdown;
    use super::*;

    fn txt(md: &str) -> String {
        render_text(&parse_markdown(md))
    }

    #[test]
    fn markup_is_removed() {
        assert_eq!(
            txt("## Beslutninger\n\nVi **lanserer** på *fredag* med `v2`."),
            "Beslutninger\n\nVi lanserer på fredag med v2."
        );
    }

    #[test]
    fn lists_get_bullets_and_hanging_indents() {
        assert_eq!(
            txt("- **Eier:** Hege\n- To\n  - Under\n\n3. tre\n4. fire"),
            "• Eier: Hege\n• To\n  ◦ Under\n\n3. tre\n4. fire"
        );
    }

    #[test]
    fn tables_are_padded_into_columns() {
        assert_eq!(
            txt("| Hva | Hvem |\n|---|---|\n| Lansering | Michael |\n| QA | Hege |"),
            "Hva        Hvem\n---------  -------\nLansering  Michael\nQA         Hege"
        );
    }

    #[test]
    fn quotes_code_and_rules() {
        assert_eq!(
            txt("> sitat\n\n```\nfn a() {}\n```\n\n---\n\nslutt"),
            "> sitat\n\n    fn a() {}\n\n* * *\n\nslutt"
        );
    }
}
