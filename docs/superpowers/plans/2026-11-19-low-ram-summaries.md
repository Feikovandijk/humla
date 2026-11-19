# Local summaries on 8 GB Macs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Whole-meeting on-device summaries on an 8 GB Mac (MacBook Neo) without Ollama running out of memory.

**Architecture:** A RAM-derived context cap bounds Ollama's `num_ctx`. When a local summary prompt would exceed the cap, `run_summary` splits the transcript at line boundaries, summarizes each piece with a fixed map prompt, then runs the preset prompt over the piece summaries (map-reduce). Cloud providers are untouched.

**Tech Stack:** Rust (Tauri 2, tokio, `libc` already a dependency), React/TypeScript.

**Spec:** `docs/superpowers/specs/2026-11-19-low-ram-summaries-design.md`

## Global Constraints

- Context cap by physical RAM: ≤ 8 GiB → 8192, ≤ 16 GiB → 32768, else 65536. Unknown RAM → 65536 (today's behaviour).
- Token estimate stays chars / 4 (the existing heuristic in `ollama_chat_stream`).
- Output budget stays `num_predict` = 8192 with think, 4096 without; plus the existing 512-token margin.
- Only the `ProviderId::Local` summary path splits. OpenAI / Anthropic paths unchanged.
- Map calls run sequentially (RAM), never in parallel, and always with `think: false`.
- The reduce call keeps `[Notater]` and the label `[Transkripsjon]`; an empty side is still `(ingen)`.
- 8 GB recommended model: `qwen3.5:2b` (official Ollama tag; Qwen family, so the existing Qwen sampling profile applies).
- Mirrored-copy rule: Settings and onboarding read the tier from `src/lib/localModels.ts`; no second list.

## Review Focus

1. A transcript with one huge line (unlabelled fallback transcript, no newlines) — expect it split at whitespace, no text dropped. Test in Task 2.
2. Norwegian text (æøå, multi-byte) at a piece boundary — expect no panic on a char boundary. Test in Task 2.
3. Typed notes plus system prompt alone bigger than the budget — expect a piece budget floor, no zero-size pieces or endless loop. Test in Task 2 (`piece_budget`).
4. Think mode on at 8 GB — expect think forced off so the output budget doesn't eat the whole context. Test in Task 1.
5. Piece summaries that together still don't fit — expect at most 3 reduce rounds, then the final call runs anyway. Reviewed in Task 3 (round cap constant).

---

### Task 1: RAM-derived context cap

**Files:**
- Modify: `src-tauri/src/openai.rs` (`ollama_chat_stream`, ~1055-1085; tests module ~1860)

**Interfaces:**
- Produces (all `pub(crate)` in `openai.rs`):
  - `fn ctx_cap_for_ram(bytes: Option<u64>) -> usize`
  - `fn local_ctx_cap() -> usize` — `OnceLock`, reads `hw.memsize` via `libc::sysctlbyname`, passes `None` on failure.
  - `fn effective_think(think: bool, cap: usize) -> bool` — `think && cap > 8192`.
  - `fn num_predict(think: bool) -> usize` — 8192 / 4096.
  - `fn input_budget_chars(cap: usize, think: bool) -> usize` — `(cap - num_predict(effective_think(think, cap)) - 512) * 4`, saturating.

- [ ] **Step 1: Write failing tests** in the `openai.rs` tests module:

```rust
#[test]
fn ram_tiers_map_to_context_caps() {
    const GIB: u64 = 1 << 30;
    assert_eq!(ctx_cap_for_ram(Some(8 * GIB)), 8192);
    assert_eq!(ctx_cap_for_ram(Some(16 * GIB)), 32768);
    assert_eq!(ctx_cap_for_ram(Some(24 * GIB)), 65536);
    assert_eq!(ctx_cap_for_ram(None), 65536);
}

#[test]
fn think_is_forced_off_at_the_smallest_tier() {
    assert!(!effective_think(true, 8192));
    assert!(effective_think(true, 32768));
    assert!(!effective_think(false, 65536));
}

#[test]
fn input_budget_leaves_room_for_output() {
    assert_eq!(input_budget_chars(8192, true), (8192 - 4096 - 512) * 4);
    assert_eq!(input_budget_chars(32768, true), (32768 - 8192 - 512) * 4);
}
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test --lib ram_tiers think_is_forced input_budget` — expect compile failure (functions undefined).
- [ ] **Step 3: Implement** the five functions; in `ollama_chat_stream` replace the local `num_predict` with `num_predict(think)` after `let think = effective_think(think, local_ctx_cap());`, and bound the doubling loop by `local_ctx_cap()` instead of the literal 65536. Update the `OllamaOptions.num_ctx` comment's bound.
- [ ] **Step 4: Run** the same tests plus `cargo test --lib sampling` — expect PASS.
- [ ] **Step 5: Commit** `feat(summary): cap local context by the Mac's RAM`.

### Task 2: Transcript splitter

**Files:**
- Create: `src-tauri/src/commands/summary_split.rs` (declare `mod summary_split;` in `src-tauri/src/commands.rs` next to `mod summary;`, not re-exported)

**Interfaces:**
- Produces (`pub(super)`):
  - `const MIN_PIECE_CHARS: usize = 4000;`
  - `fn piece_budget(budget_chars: usize, fixed_chars: usize) -> usize` — `budget_chars.saturating_sub(fixed_chars).max(MIN_PIECE_CHARS)`.
  - `fn split_transcript(text: &str, max_chars: usize) -> Vec<String>` — packs whole lines (joined by `\n`) while ≤ `max_chars` bytes; a line longer than `max_chars` is cut at the last whitespace at or before the limit (on a char boundary; hard cut on a char boundary if there is none).

- [ ] **Step 1: Write failing tests** in the new file:

```rust
fn words(s: &str) -> Vec<&str> { s.split_whitespace().collect() }

#[test]
fn pieces_fit_and_keep_every_word_in_order() {
    let text = (0..200).map(|i| format!("Speaker {}: line number {i}", i % 3)).collect::<Vec<_>>().join("\n");
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
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test --lib summary_split` — expect compile failure.
- [ ] **Step 3: Implement** both functions.
- [ ] **Step 4: Run** `cargo test --lib summary_split` — expect PASS.
- [ ] **Step 5: Commit** `feat(summary): split long transcripts into pieces`.

### Task 3: Map-reduce in `run_summary`

**Files:**
- Modify: `src-tauri/src/commands/summary.rs` (`ResolvedProvider` gains `#[derive(Clone)]`; `run_summary` ~274-350)

**Interfaces:**
- Consumes: `openai::{local_ctx_cap, input_budget_chars}` (Task 1), `summary_split::{split_transcript, piece_budget}` (Task 2), existing `language_directive`, `ResolvedProvider::complete`.
- Produces: `const MAX_REDUCE_ROUNDS: usize = 3;` and `fn map_prompt(language: &str) -> String` in `summary.rs`.

- [ ] **Step 1: Implement `map_prompt`** — exact copy, then the language directive appended:

```text
You are given one consecutive part of a meeting transcript, or notes on consecutive parts of one. Write concise bullet-point notes: key points, decisions, action items with owners, and any names, numbers and dates. Keep speaker names as written. Do not add anything that is not in the text.
```

- [ ] **Step 2: Wire the split** in `run_summary`, after `full_prompt` is built and before the single `complete` call. Only when `provider.provider == ProviderId::Local`:
  - `budget = input_budget_chars(local_ctx_cap(), provider.think)`.
  - If `full_prompt.len() + user_message.len() <= budget`: unchanged single call.
  - Otherwise: `mapper = ResolvedProvider { think: false, ..provider.clone() }`; `text = note.transcript.clone()`; loop up to `MAX_REDUCE_ROUNDS`: `pieces = split_transcript(&text, piece_budget(budget, map_prompt.len()))`; for each piece `i`, emit a `summary_thinking_delta` of `"Summarizing part {i+1} of {n}…\n"` (through the same `on_chunk` closure as `StreamChunk::Thinking`), then `mapper.complete(&map_prompt, piece, |_| {})`; `text = results.join("\n\n")`; break when `full_prompt.len() + notes_block.len() + text.len() + 64 <= budget`.
  - Reduce: `user_message = format!("{notes_block}\n\n[Transkripsjon]\n(notes on consecutive parts of the transcript, in order)\n{text}")`, then the existing streamed `provider.complete(&full_prompt, &user_message, on_chunk)`.
  - `on_chunk` is `move`; make it a `let mut on_chunk` and pass `&mut on_chunk` to the progress emit, or clone the `app`/`note_id` pair for a second small closure — implementer's choice.
  - One `eprintln!("[llm] summary split: {n} pieces, round {r}")` per round.
- [ ] **Step 3: Run** `cd src-tauri && cargo test --lib summary && cargo clippy --lib -- -D warnings` — expect PASS, no warnings.
- [ ] **Step 4: Manual check** (optional when no 8 GB Mac is at hand): temporarily force `local_ctx_cap()` to 8192, summarize a note with a ≥ 1 h transcript on Ollama with `qwen3.5:2b`; expect "Summarizing part N of M…" lines in the thinking trace, then a streamed summary covering the end of the meeting. Revert the force.
- [ ] **Step 5: Commit** `feat(summary): summarize long local transcripts in pieces`.

### Task 4: 8 GB model tier in the UI

**Files:**
- Modify: `src/lib/localModels.ts`, `src/pages/settings/tabs/Summary.tsx:44-47`, `src/pages/onboarding/steps/Summary.tsx` (~583-586 and the `ModelSelect` label at ~617)

**Interfaces:**
- Produces: `export const RECOMMENDED_OLLAMA_MODEL_8GB = "qwen3.5:2b";` appended last to `RECOMMENDED_OLLAMA_MODELS`.

- [ ] **Step 1: Add the constant** and the array entry; extend the header comment with one line: 8 GB Macs (MacBook Neo) run `qwen3.5:2b`, and long meetings are summarized in pieces (`summary_split.rs`).
- [ ] **Step 2: Copy** — Settings hint becomes: `On a 16 GB Mac use <code>ollama pull {16GB}</code>, on an 8 GB Mac <code>ollama pull {8GB}</code>. {RECOMMENDED} needs ~24 GB+.` Onboarding hint gets the same 8 GB clause. `ModelSelect` labels `RECOMMENDED_OLLAMA_MODEL_8GB` as ` (8 GB Macs)`. No em dashes in new copy.
- [ ] **Step 3: Run** `pnpm lint && pnpm build && pnpm test --run` — expect 0 lint errors, build passes, tests pass.
- [ ] **Step 4: Commit** `feat(summary): recommend qwen3.5:2b on 8 GB Macs`.
