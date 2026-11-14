use super::{DEFAULT_LANGUAGE, languages};

/// sys-locale reads navigator.languages on the web and OS language preferences
/// on desktop. Only call this when there are no saved inspector preferences.
pub(super) fn detected_language() -> &'static str {
    preferred_language(sys_locale::get_locales())
}

fn preferred_language(locales: impl IntoIterator<Item = impl AsRef<str>>) -> &'static str {
    locales
        .into_iter()
        .find_map(|locale| supported_language(locale.as_ref()))
        .unwrap_or(DEFAULT_LANGUAGE)
}

fn supported_language(locale: &str) -> Option<&'static str> {
    // Accept both BCP 47 browser tags and POSIX locale spellings.
    let normalized = locale
        .trim()
        .split(['.', '@'])
        .next()?
        .replace('_', "-")
        .to_ascii_lowercase();
    let supported = languages();
    let mut candidate = normalized.as_str();
    loop {
        if let Some(&(code, _)) = supported
            .iter()
            .find(|&&(code, _)| code.replace('_', "-").eq_ignore_ascii_case(candidate))
        {
            return Some(code);
        }
        let Some((parent, _)) = candidate.rsplit_once('-') else {
            break;
        };
        candidate = parent;
    }

    // The catalog uses zh_CN, while browsers/OSes can prefer a script tag or
    // bare zh. Never apply Simplified Chinese to a Traditional Chinese locale.
    let parts: Vec<_> = normalized.split('-').collect();
    if parts.first() == Some(&"zh")
        && !parts.contains(&"hant")
        && (parts.contains(&"hans")
            || (!parts.iter().any(|part| ["tw", "hk", "mo"].contains(part))
                && (parts.len() == 1 || parts.contains(&"cn") || parts.contains(&"sg"))))
    {
        return supported
            .iter()
            .find_map(|&(code, _)| (code == "zh_CN").then_some(code));
    }

    // Each remaining language has a single regional catalog (en_US, pt_BR,
    // es_419), so it also serves the bare language and its other regions
    // instead of falling back to English. Chinese is resolved above.
    let primary = *parts.first()?;
    if primary == "zh" {
        return None;
    }
    supported
        .iter()
        .find_map(|&(code, _)| (code.split('_').next() == Some(primary)).then_some(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_supported_browser_and_os_locale_variants() {
        for locale in ["en", "en-US", "EN_gb.UTF-8", "en-CA", "en-US-u-hc-h12"] {
            assert_eq!(preferred_language([locale]), "en_US", "{locale}");
        }
        for locale in [
            "zh",
            "zh_CN",
            "zh-CN",
            "zh_CN.UTF-8",
            "zh-Hans",
            "zh-Hans-CN",
            "zh-SG",
        ] {
            assert_eq!(preferred_language([locale]), "zh_CN", "{locale}");
        }
        for locale in ["pt", "pt_BR", "pt-BR", "pt_BR.UTF-8", "pt-PT", "pt-AO"] {
            assert_eq!(preferred_language([locale]), "pt_BR", "{locale}");
        }
        for locale in ["vi", "vi-VN", "vi_VN", "vi_VN.UTF-8"] {
            assert_eq!(preferred_language([locale]), "vi_VN", "{locale}");
        }
        for locale in ["es", "es_ES", "es-ES", "es_MX.UTF-8", "es-419", "es-CO", "ES-ar"] {
            assert_eq!(preferred_language([locale]), "es_419", "{locale}");
        }
    }

    #[test]
    fn unsupported_and_traditional_chinese_locales_fall_back_to_english() {
        for locale in [
            "fr-CA",
            "de",
            "zh-TW",
            "zh-HK",
            "zh-MO",
            "zh-Hant",
            "zh-Hant-TW",
            "C",
            "",
        ] {
            assert_eq!(preferred_language([locale]), "en_US", "{locale}");
        }
        assert_eq!(preferred_language(std::iter::empty::<&str>()), "en_US");
    }

    #[test]
    fn uses_the_first_supported_language_in_preference_order() {
        assert_eq!(preferred_language(["fr-CA", "zh-Hans", "en-US"]), "zh_CN");
        assert_eq!(preferred_language(["en-CA", "zh-CN"]), "en_US");
        assert_eq!(preferred_language(["zh-Hant", "zh-Hans"]), "zh_CN");
        assert_eq!(preferred_language(["fr-CA", "pt-BR", "en-US"]), "pt_BR");
        assert_eq!(preferred_language(["fr-CA", "vi-VN", "en-US"]), "vi_VN");
        assert_eq!(preferred_language(["en-US", "pt-BR"]), "en_US");
        assert_eq!(preferred_language(["fr-CA", "es-MX", "en-US"]), "es_419");
        assert_eq!(preferred_language(["es-419", "pt-BR"]), "es_419");
        assert_eq!(preferred_language(["en-US", "es-ES"]), "en_US");
    }
}
