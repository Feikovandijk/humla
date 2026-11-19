# Local summaries on 8 GB Macs (MacBook Neo)

## Intent
A MacBook Neo (8 GB unified memory) should produce an on-device summary of a
meeting of any length without Ollama running out of memory. Today the local
path sizes `num_ctx` up to 64K (`openai.rs`, `ollama_chat_stream`), whose KV
cache alone can exceed what an 8 GB Mac has free, and no model tier targets 8 GB.

## Goals / non-goals
- Goal: whole-meeting local summaries on 8 GB, no setting to get wrong.
- Goal: unchanged behaviour wherever the prompt already fits.
- Non-goal: cloud providers (OpenAI, Anthropic, Groq…) — untouched.
- Non-goal: chat and automatic titles — untouched (titles already sample the transcript).
- Anti-goal: a summary that silently drops part of the meeting.

## Approaches
- **A. RAM-derived context cap + map-reduce when over it (picked).** No setting;
  only kicks in when needed; also protects 16 GB Macs on very long meetings.
- B. "Low memory mode" toggle, default on at 8 GB — one more setting, and turning
  it off on a Neo crashes Ollama.
- C. Always split local summaries above a fixed size — needlessly worse/slower on big Macs.

## Design
- **RAM tier:** read `hw.memsize` once (sysctl). Context cap for local summaries:
  ≤ 8 GB → 8192, ≤ 16 GB → 32768, else 65536 (today's max).
- **Cap applies** to `ollama_chat_stream`'s adaptive `num_ctx` (upper bound) on the
  summary path.
- **Split decision** in `run_summary`, local provider only: estimate tokens
  (chars / 4, the existing heuristic) of system prompt + user message + output
  budget. If it fits the cap → single call, as today.
- **Map:** split `note.transcript` at line (speaker-turn) boundaries into pieces
  that fit the cap with the map prompt and output budget. Each piece gets a fixed,
  language-directed prompt: extract key points, decisions, action items, keeping
  speaker names. Not streamed to the content panel.
- **Reduce:** the preset prompt as today, with `[Notater]` unchanged and
  `[Transkripsjon]` replaced by the piece summaries in order. Streams as today.
  If the joined piece summaries still exceed the cap, reduce them in groups again.
- **Progress:** before each map call emit a thinking delta
  "Summarizing part N of M…".
- **Model tier:** add an 8 GB recommendation to `src/lib/localModels.ts`
  (`RECOMMENDED_OLLAMA_MODELS` gains a third entry) and mention it in Settings →
  Summary and onboarding next to the 16 GB line.

## Left to the builder
- The exact 8 GB Ollama tag (verify in the Ollama library; candidate: a ~2B Qwen 3.5
  or small Gemma) and whether `sampling_profile` already covers its family.
- Map prompt wording; one-piece-per-call ordering (sequential, not parallel — RAM).
- A unit test for the splitter (pieces fit, cover every line, keep order) and the
  RAM → cap mapping.
