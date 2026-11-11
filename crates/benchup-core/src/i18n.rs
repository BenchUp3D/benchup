use std::cell::Cell;

thread_local! {
    // UI and export operations resolve messages on their calling thread.
    static CHINESE: Cell<bool> = const { Cell::new(false) };
}

pub fn set_language(language: &str) {
    CHINESE.set(matches!(language, "zh_CN" | "zh-CN" | "zh-Hans"));
}

/// Start an operation with the language selected by its caller.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_localized<F, T>(operation: F) -> std::thread::JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let chinese = is_chinese();
    std::thread::spawn(move || {
        CHINESE.set(chinese);
        operation()
    })
}

pub fn is_chinese() -> bool {
    CHINESE.get()
}

include!(concat!(env!("OUT_DIR"), "/messages.rs"));

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
        super::set_language("en");
        assert_eq!(tr!("common.cancel"), "Cancel");
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
