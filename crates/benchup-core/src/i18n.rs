use std::cell::Cell;

/// A bundled message catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    SimplifiedChinese,
    BrazilianPortuguese,
    Vietnamese,
    Spanish,
    Italian,
}

thread_local! {
    // UI and export operations resolve messages on their calling thread.
    static LANGUAGE: Cell<Language> = const { Cell::new(Language::English) };
}

pub fn set_language(language: &str) {
    LANGUAGE.set(match language {
        "zh_CN" | "zh-CN" | "zh-Hans" => Language::SimplifiedChinese,
        "pt_BR" | "pt-BR" => Language::BrazilianPortuguese,
        "vi_VN" | "vi-VN" => Language::Vietnamese,
        "es_419" | "es-419" => Language::Spanish,
        "it_IT" | "it-IT" => Language::Italian,
        _ => Language::English,
    });
}

/// Start an operation with the language selected by its caller.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_localized<F, T>(operation: F) -> std::thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let language = current_language();
    std::thread::spawn(move || {
        LANGUAGE.set(language);
        operation()
    })
}

pub fn current_language() -> Language {
    LANGUAGE.get()
}

pub fn is_chinese() -> bool {
    current_language() == Language::SimplifiedChinese
}

include!(concat!(env!("OUT_DIR"), "/messages.rs"));

#[cfg(test)]
pub(crate) fn catalog_message(language: &str, key: &str) -> String {
    let catalog = match language {
        "zh_CN" => include_str!("../../../assets/i18n/zh_CN.json"),
        "pt_BR" => include_str!("../../../assets/i18n/pt_BR.json"),
        "vi_VN" => include_str!("../../../assets/i18n/vi_VN.json"),
        "es_419" => include_str!("../../../assets/i18n/es_419.json"),
        "it_IT" => include_str!("../../../assets/i18n/it_IT.json"),
        "en_US" => include_str!("../../../assets/i18n/en_US.json"),
        _ => panic!("unsupported test language: {language}"),
    };
    let messages: serde_json::Value = serde_json::from_str(catalog).unwrap();
    messages[key].as_str().unwrap().to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn language_switches_labels_formats_and_plurals_without_restart() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                super::set_language("en");
            }
        }
        let _reset = Reset;
        super::set_language("zh_CN");
        assert_eq!(tr!("common.cancel"), "取消");
        assert_eq!(tr!("settings.settings"), "设置");
        for (fit, key) in [
            (crate::ImageFit::Contain, "core.reference_images.contain"),
            (crate::ImageFit::Cover, "core.reference_images.cover"),
            (crate::ImageFit::Stretch, "core.reference_images.stretch"),
        ] {
            assert_eq!(fit.label(), super::catalog_message("zh_CN", key));
        }
        assert_eq!(
            serde_json::to_string(&crate::ImageFit::Contain).unwrap(),
            "\"contain\""
        );
        #[cfg(not(target_arch = "wasm32"))]
        assert!(super::spawn_localized(super::is_chinese).join().unwrap());
        assert_eq!(tr!("settings.connected", device = "test"), "已连接：test");
        let mut count = 0;
        assert_eq!(
            tr!(
                "app.selection_entities",
                count = {
                    count += 1;
                    count
                }
            ),
            "1 个图元"
        );
        assert_eq!(count, 1);
        assert_eq!(tr!("app.selection_entities", count = 2), "2 个图元");
        super::set_language("pt_BR");
        assert_eq!(tr!("common.cancel"), "Cancelar");
        assert_eq!(tr!("settings.settings"), "Configurações");
        for (fit, key) in [
            (crate::ImageFit::Contain, "core.reference_images.contain"),
            (crate::ImageFit::Cover, "core.reference_images.cover"),
            (crate::ImageFit::Stretch, "core.reference_images.stretch"),
        ] {
            assert_eq!(fit.label(), super::catalog_message("pt_BR", key));
        }
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            super::spawn_localized(super::current_language).join().unwrap(),
            super::Language::BrazilianPortuguese
        );
        assert_eq!(tr!("app.selection_entities", count = 1), "1 entidade");
        assert_eq!(tr!("app.selection_entities", count = 2), "2 entidades");
        super::set_language("vi_VN");
        assert_eq!(tr!("common.cancel"), "Hủy");
        assert_eq!(tr!("settings.settings"), "Cài đặt");
        // Vietnamese uses the `other` form even for one selected item.
        for count in [0, 1, 2] {
            assert_eq!(
                tr!("app.selection_entities", count = count),
                format!("{count} thực thể")
            );
            assert_eq!(
                tr!("app.selection_edges", count = count),
                format!("{count} cạnh")
            );
            assert_eq!(
                tr!("app.selection_faces", count = count),
                format!("{count} mặt")
            );
            assert_eq!(
                tr!("app.selection_dimensions", count = count),
                format!("{count} kích thước")
            );
            assert_eq!(
                tr!("app.selection_guides", count = count),
                format!("{count} đường dẫn")
            );
            assert_eq!(
                tr!("app.selection_text_labels", count = count),
                format!("{count} nhãn văn bản")
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            super::spawn_localized(super::current_language).join().unwrap(),
            super::Language::Vietnamese
        );
        super::set_language("en");
        super::set_language("es_419");
        assert_eq!(tr!("common.cancel"), "Cancelar");
        assert_eq!(tr!("settings.settings"), "Configuración");
        for (fit, key) in [
            (crate::ImageFit::Contain, "core.reference_images.contain"),
            (crate::ImageFit::Cover, "core.reference_images.cover"),
            (crate::ImageFit::Stretch, "core.reference_images.stretch"),
        ] {
            assert_eq!(fit.label(), super::catalog_message("es_419", key));
        }
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            super::spawn_localized(super::current_language).join().unwrap(),
            super::Language::Spanish
        );
        assert_eq!(tr!("app.selection_entities", count = 1), "1 entidad");
        assert_eq!(tr!("app.selection_entities", count = 2), "2 entidades");
        super::set_language("es-419");
        assert_eq!(super::current_language(), super::Language::Spanish);
        super::set_language("en_US");
        assert_eq!(tr!("common.cancel"), "Cancel");
        assert_eq!(
            crate::ImageFit::Contain.label(),
            super::catalog_message("en_US", "core.reference_images.contain")
        );
        super::set_language("unsupported");
        assert_eq!(tr!("settings.settings"), "Settings");
    }

    #[test]
    fn resolves_labels_and_named_format_arguments() {
        let label: &str = tr!("common.cancel");
        assert_eq!(label, "Cancel");
        assert_eq!(
            tr!("app.tool_with_shortcut", shortcut = "L", tool = "Line"),
            "Line (L)"
        );
    }

    #[test]
    fn selects_complete_plural_messages_and_evaluates_count_once() {
        assert_eq!(tr!("app.selection_entities", count = 0), "0 entities");
        let mut count = 0;
        assert_eq!(
            tr!(
                "app.selection_entities",
                count = {
                    count += 1;
                    count
                }
            ),
            "1 entity"
        );
        assert_eq!(count, 1);
        assert_eq!(
            tr!("cutting_plan.stock_required", count = 2, kerf = "3 mm"),
            "Stock required: 2 stock pieces | Saw kerf: 3 mm"
        );
    }
}
