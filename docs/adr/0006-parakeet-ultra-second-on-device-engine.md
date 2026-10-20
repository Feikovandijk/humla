# ADR-0006: Parakeet Ultra joins Whisper as a second on-device ASR engine; NB Whisper is retired

- **Status:** Accepted
- **Date:** 2026-10-20
- **Supersedes:** the NB Whisper entry in the local model registry (not a prior ADR)

## Context

Humla's on-device transcription was whisper.cpp only: eight multilingual GGML models were the
candidate set, and the Norwegian-finetuned NB Whisper was the language-specific exception. Whisper
is the right generalist — 99 languages, word timestamps, a prompt slot for custom vocabulary and
cross-chunk context — but its autoregressive decoder is slow, and 2026's transducer models beat it
on English and major European languages at a fraction of the compute.

The user asked for a curated local catalog and for the best model a 48 GB M4 can run. That machine
can hold full-precision `large-v3` (3.1 GB) comfortably, but the fastest accurate option is
NVIDIA's Parakeet, which FluidAudio already ships as CoreML for the Neural Engine. FluidAudio is
already a pinned dependency for diarization, so the marginal runtime cost is one more mode in the
existing sidecar, not a new binary.

## Decision

Ship **nine** on-device choices:

1. Eight multilingual whisper.cpp GGML models: `tiny-q5`, `base-q5`, `small-q5`, `medium-q5`,
   `large-v2-q5`, `large-v3-q5`, `large-v3-turbo-q5` (default), and full-precision `large-v3`
   (quality ceiling on a 48 GB Mac).
2. **Parakeet Ultra** (`AsrModelVersion.ultra`) as a separate provider, selectable explicitly,
   never the fresh-install default.

Retire NB Whisper completely: no registry entry, no onboarding route, no per-language default.
A one-shot migration rewrites a stored NB default to `large-v3-turbo-q5`, drops NB per-language
overrides, and deletes the orphaned model file.

## Why Ultra, not Parakeet TDT v3

Ultra is moondream's full-precision post-training of v3. Same architecture, same 25 languages, same
API, same speed; better WER on English and on all 24 measured FLEURS languages (LibriSpeech
test-clean 2.13% vs 2.27%; FLEURS 24-language mean 11.67% vs 14.81%). There is no reason to ship v3
when Ultra is a drop-in.

## Considered options

- **Whisper only, with full `large-v3` as the 8th model.** Rejected: it leaves the fastest accurate
  English/European option unused, and Parakeet's ANE path costs no new signed binary.
- **Parakeet TDT v3.** Rejected in favour of Ultra — same size class and speed, worse accuracy.
- **Canary-Qwen 2.5B.** Best English WER on the Open ASR Leaderboard (~5.6%), but English-only and
  NeMo-first; a poor Mac integration and no multilingual coverage.
- **Cohere Transcribe 2B.** 14 languages, no Norwegian, and a 3.5 GB-class CoreML stack; no
  advantage over Ultra for this app.
- **Qwen3-ASR 1.7B via MLX.** The strongest multilingual challenger (52 languages, word timestamps,
  MLX ports exist), but it needs a Python/MLX sidecar and a new model lifecycle. Deferred, not
  rejected forever; the `ProviderConfig` tag makes it a later addition rather than a rewrite.
- **Keep NB Whisper as a ninth, gated entry.** Rejected by product decision: Norwegian-specific
  routing is out of scope. The cost is real — NB was the only Norwegian-tuned model, and generic
  Whisper is weaker on Norwegian — and is recorded here rather than hidden.

## Consequences

- **Two on-device engines, one sidecar.** `speaker-diarize` gains `asr-status` / `asr-download` /
  `asr-delete` / `asr-serve`. `asr-serve` is long-lived because a per-chunk process would reload
  600 MB every chunk; it exits on stdin EOF, which is how the app's `_exit(0)` reaps it.
- **Language scope is enforced in the adapter.** Parakeet covers 25 European languages and no
  Norwegian; `stt/parakeet.rs` refuses `no`/`nb`/`nn` and any code outside that set with a message
  pointing at Whisper, rather than emitting nonsense.
- **Parakeet gives up Whisper's bias features.** No initial prompt, no prior-context continuation,
  no detected-language report. Custom vocabulary and cross-chunk continuity remain Whisper-only.
- **Full `large-v3` is a 3.1 GB download.** Fine on a 48 GB Mac; the picker labels it as the
  best-quality option, not the default.
- **The mirror stays intact.** `providers.rs` gains `parakeet`, mirrored by `providers.ts`; the
  registry test keeps the two honest.
