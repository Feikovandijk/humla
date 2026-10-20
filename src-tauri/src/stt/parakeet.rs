//! Parakeet Ultra batch STT adapter. The heavy lifting lives in
//! `crate::parakeet`'s long-lived sidecar; this maps its words into the
//! shared `TranscribeResult`.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

use crate::parakeet::{self, ParakeetManager};
use crate::stt::adapter::{BatchSttAdapter, TranscribeCtx, TranscribeResult};

pub struct ParakeetAdapter {
    manager: Arc<ParakeetManager>,
}

impl ParakeetAdapter {
    pub fn new(manager: Arc<ParakeetManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl BatchSttAdapter for ParakeetAdapter {
    fn provider_id(&self) -> &'static str {
        "parakeet"
    }

    fn label(&self) -> &'static str {
        "Parakeet Ultra"
    }

    fn supports_language(&self, lang: &str) -> bool {
        parakeet::supports_language(lang)
    }

    fn supports_word_timestamps(&self, _model: &str) -> bool {
        true
    }

    async fn transcribe(
        &self,
        ctx: TranscribeCtx<'_>,
        audio: &Path,
    ) -> Result<TranscribeResult> {
        if !ctx.language.is_empty() && !parakeet::supports_language(ctx.language) {
            return Err(anyhow!(
                "Parakeet Ultra does not support language '{}'. Choose a Whisper model in Settings → Transcription.",
                ctx.language
            ));
        }
        // Parakeet has no Whisper-style prompt or cross-chunk continuation:
        // ctx.bias_terms and ctx.prior_context are deliberately unused.
        let result = self.manager.transcribe(audio).await?;
        Ok(TranscribeResult {
            text: result.text,
            words: result
                .words
                .into_iter()
                .map(|w| crate::stt::Word {
                    text: w.text,
                    start_ms: w.start_ms,
                    end_ms: w.end_ms,
                })
                .collect(),
            detected_language: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_and_language_scope() {
        let a = ParakeetAdapter::new(Arc::new(ParakeetManager::new(None)));
        assert_eq!(a.provider_id(), "parakeet");
        assert_eq!(a.label(), "Parakeet Ultra");
        assert!(a.supports_word_timestamps("ultra"));
        assert!(a.supports_language("en"));
        assert!(a.supports_language("auto"));
        assert!(!a.supports_language("no"));
    }
}
