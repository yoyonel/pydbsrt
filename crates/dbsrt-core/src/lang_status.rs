//! Language detection confidence levels for subtitle coupling certification.
//!
//! A ground-truth system must never silently guess the language of a subtitle.
//! When language cannot be determined with high confidence, the coupling is
//! certified with lang="und" (ISO 639-2 undetermined) and the status reflects
//! the uncertainty level, allowing downstream search to filter accordingly.

use serde::{Deserialize, Serialize};

/// Confidence level of a subtitle's language attribution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum LangStatus {
    /// Language explicitly provided by the user via --lang CLI flag.
    /// Highest confidence. Always trusted.
    Confirmed(String),

    /// Language detected from a valid ISO 639-1/639-2 suffix in the filename
    /// (e.g. ".fr.srt", ".fre.srt", ".en.srt"). High confidence.
    Detected(String),

    /// Language inferred by heuristic fallback (DEFAULT_SUBTITLE_LANG="en").
    /// LOW confidence — POSSIBLY WRONG. Kept for backward compat with old
    /// couplings.json entries but never produced by new code.
    Inferred(String),

    /// Language could not be determined. Coupling is certified with lang="und"
    /// (ISO 639-2 undetermined). User must relabel later via CLI command
    /// (future: dbsrt coupling:relabel).
    #[default]
    Unknown,
}

impl LangStatus {
    /// Returns the ISO 639 code to write into the `lang` field of
    /// CertifiedSubtitleRecord. Always a valid 2-3 letter lowercase code,
    /// never "auto" or empty.
    pub fn to_lang_code(&self) -> String {
        match self {
            LangStatus::Confirmed(code) => code.clone(),
            LangStatus::Detected(code) => code.clone(),
            LangStatus::Inferred(code) => code.clone(),
            LangStatus::Unknown => "und".to_string(),
        }
    }

    /// True if the language attribution is certain (Confirmed or Detected).
    /// Used by search to filter out uncertain couplings on explicit --lang queries.
    pub fn is_certain(&self) -> bool {
        matches!(self, LangStatus::Confirmed(_) | LangStatus::Detected(_))
    }

    /// True if the language attribution is uncertain (Inferred or Unknown).
    /// These couplings should only be returned by search when --include-unknown
    /// or --lang auto is specified.
    pub fn is_uncertain(&self) -> bool {
        matches!(self, LangStatus::Inferred(_) | LangStatus::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_lang_code_confirmed() {
        assert_eq!(LangStatus::Confirmed("fr".to_string()).to_lang_code(), "fr");
        assert_eq!(LangStatus::Confirmed("en".to_string()).to_lang_code(), "en");
    }

    #[test]
    fn test_to_lang_code_detected() {
        assert_eq!(LangStatus::Detected("es".to_string()).to_lang_code(), "es");
    }

    #[test]
    fn test_to_lang_code_inferred() {
        // Legacy case: keep the code but mark as uncertain
        assert_eq!(LangStatus::Inferred("en".to_string()).to_lang_code(), "en");
    }

    #[test]
    fn test_to_lang_code_unknown() {
        assert_eq!(LangStatus::Unknown.to_lang_code(), "und");
    }

    #[test]
    fn test_is_certain() {
        assert!(LangStatus::Confirmed("fr".to_string()).is_certain());
        assert!(LangStatus::Detected("en".to_string()).is_certain());
        assert!(!LangStatus::Inferred("en".to_string()).is_certain());
        assert!(!LangStatus::Unknown.is_certain());
    }

    #[test]
    fn test_is_uncertain() {
        assert!(!LangStatus::Confirmed("fr".to_string()).is_uncertain());
        assert!(!LangStatus::Detected("en".to_string()).is_uncertain());
        assert!(LangStatus::Inferred("en".to_string()).is_uncertain());
        assert!(LangStatus::Unknown.is_uncertain());
    }

    #[test]
    fn test_serde_roundtrip_confirmed() {
        let s = LangStatus::Confirmed("fr".to_string());
        let json = serde_json::to_string(&s).unwrap();
        let back: LangStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn test_serde_roundtrip_unknown() {
        let s = LangStatus::Unknown;
        let json = serde_json::to_string(&s).unwrap();
        // Unknown variant should not have a "value" field
        assert!(!json.contains("value"));
        let back: LangStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
