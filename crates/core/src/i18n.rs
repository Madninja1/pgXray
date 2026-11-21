//! Locale selection for presentation-only backend text.
//! Stable IDs, SQL, saved identifiers and diagnostic thresholds must never be localized.
use std::sync::atomic::{AtomicU8, Ordering};

const EN: u8 = 0;
const RU: u8 = 1;
static BACKEND_LOCALE: AtomicU8 = AtomicU8::new(RU);

/// Single-window desktop locale. Tauri updates this after loading the UI preference.
pub fn set_locale(value: &str) -> Result<(), String> {
    let next = match value {
        "en" => EN,
        "ru" => RU,
        _ => return Err("Unsupported locale (expected en or ru)".into()),
    };
    BACKEND_LOCALE.store(next, Ordering::Release);
    Ok(())
}

/// Choose text for transient user-facing output, never for machine-readable states.
#[inline]
pub fn text(ru: &'static str, en: &'static str) -> &'static str {
    if BACKEND_LOCALE.load(Ordering::Acquire) == RU {
        ru
    } else {
        en
    }
}

/// Whether the current backend locale is Russian.
#[inline]
pub fn is_ru() -> bool {
    BACKEND_LOCALE.load(Ordering::Acquire) == RU
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_language_without_mutating_locale() {
        let before = BACKEND_LOCALE.load(Ordering::Acquire);
        assert!(set_locale("de").is_err());
        assert_eq!(BACKEND_LOCALE.load(Ordering::Acquire), before);
    }
}
