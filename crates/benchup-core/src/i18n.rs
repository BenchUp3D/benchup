include!(concat!(env!("OUT_DIR"), "/messages.rs"));

#[cfg(test)]
mod tests {
    #[test]
    fn resolves_labels_and_named_format_arguments() {
        const LABEL: &str = tr!("common.cancel");
        assert_eq!(LABEL, "Cancel");
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
