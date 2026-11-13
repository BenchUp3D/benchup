use serde_json::Value;
use std::{collections::BTreeMap, env, fmt::Write, fs, path::PathBuf};

type Catalog = BTreeMap<String, Value>;

const LANGUAGE: &str = "$crate::i18n::Language";

// Built-in edit names may be saved in any supported language by older clients.
// Keep their serialized text intact (including signed attribution), and resolve
// recognized names for the current language only when displaying history.
const HISTORY_LABEL_KEYS: &[&str] = &[
    "app.assign_stock_material",
    "app.clear_color_2",
    "app.create_component",
    "app.draw_arc",
    "app.draw_circle",
    "app.draw_freehand",
    "app.draw_line",
    "app.draw_polygon",
    "app.draw_rectangle",
    "app.erase_geometry",
    "app.erase_selection",
    "app.make_group",
    "app.move_copy",
    "app.paint_face",
    "app.paint_group",
    "app.paste",
    "cli.agent_edit",
    "construction_guides.delete_guides",
    "context_menu.clean_aligned_vertices",
    "context_menu.duplicate",
    "context_menu.explode",
    "context_menu.hide",
    "context_menu.lock",
    "context_menu.rename_2",
    "context_menu.ungroup",
    "context_menu.unhide",
    "context_menu.unlock",
    "core.solids.combine",
    "core.solids.intersect",
    "core.solids.split",
    "core.solids.trim",
    "core.solids.union",
    "fasteners.create_fasteners",
    "fasteners.edit_fasteners",
    "grain_direction.assign",
    "grain_direction.clear_history",
    "image_tools.add_reference_image",
    "image_tools.edit_reference_image",
    "image_tools.replace_reference_image",
    "joinery.create_dynamic_wood_joint",
    "joinery.edit_dynamic_wood_joint",
    "measurement_tools.add_angle_dimension",
    "measurement_tools.add_dimension",
    "measurement_tools.add_guide",
    "measurement_tools.adjust_angle_dimension",
    "measurement_tools.adjust_guide",
    "outliner.hide",
    "outliner.move_objects",
    "outliner.unhide",
    "projects.import_geometry",
    "projects.import_sketchup",
    "router.applied",
    "solid_tools.flip_x",
    "solid_tools.flip_y",
    "solid_tools.flip_z",
    "tag_menu.add_tag",
    "tag_menu.assign_tag",
    "tag_menu.delete_tag",
    "tag_menu.edit_tags",
    "tag_menu.hide_tag",
    "tag_menu.rename_tag",
    "tag_menu.show_tag",
    "tag_menu.tag_color",
    "tag_menu.unassign_tag",
    "text_tools.add_text",
    "text_tools.edit_text",
    "time_estimates.apply_materials_to_children",
    "time_estimates.change_sanding_and_finishing",
    "tools.move",
    "tools.offset",
    "tools.push_pull",
    "tools.rotate",
    "tools.scale",
    "wood_glue.applied",
    "wood_glue.contacts_applied",
    "wood_glue.removed",
];

struct Translation {
    variant: &'static str,
    catalog: Catalog,
    /// Whether the language uses the `one` form for a count of exactly 1.
    singular: bool,
}

fn read_catalog(path: &str, name: &str) -> Catalog {
    println!("cargo:rerun-if-changed={path}");
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read the {name} message catalog: {error}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse the {name} message catalog: {error}"))
}

fn plural(one: &str, other: &str, singular: bool) -> String {
    let format = |text: &str| format!("::std::format!({text:?}, count = count $(, $($args)*)?)");
    if singular {
        format!("if count as f64 == 1.0 {{ {} }} else {{ {} }}", format(one), format(other))
    } else {
        format(other)
    }
}

fn main() {
    let messages = read_catalog("../../assets/i18n/en.json", "English");
    let translations = [
        Translation {
            variant: "SimplifiedChinese",
            catalog: read_catalog("../../assets/i18n/zh_CN.json", "Chinese"),
            singular: false,
        },
        Translation {
            variant: "BrazilianPortuguese",
            catalog: read_catalog("../../assets/i18n/pt_BR.json", "Brazilian Portuguese"),
            singular: true,
        },
    ];

    // Generate every format string so Rust continues checking named arguments.
    let mut code = String::from(
        "#[doc = \"Look up an application message in the selected bundled catalog.\"]\n\
         #[macro_export]\nmacro_rules! tr {\n",
    );
    for (key, message) in &messages {
        match message {
            Value::String(text) => {
                let mut labels = String::new();
                let mut templates = String::new();
                for translation in &translations {
                    let translated = translation
                        .catalog
                        .get(key)
                        .unwrap_or(message)
                        .as_str()
                        .expect("matching message types");
                    let variant = translation.variant;
                    write!(labels, "{LANGUAGE}::{variant} => {translated:?}, ").unwrap();
                    write!(
                        templates,
                        "{LANGUAGE}::{variant} => ::std::format!({translated:?}, $($args)+), "
                    )
                    .unwrap();
                }
                writeln!(
                    code,
                    "    ({key:?}) => {{ match $crate::i18n::current_language() {{ {labels}{LANGUAGE}::English => {text:?} }} }};"
                )
                .unwrap();
                if text.contains('{') {
                    writeln!(
                        code,
                        "    ({key:?}, $($args:tt)+) => {{ match $crate::i18n::current_language() {{ {templates}{LANGUAGE}::English => ::std::format!({text:?}, $($args)+) }} }};"
                    )
                    .unwrap();
                }
            }
            Value::Object(forms) => {
                assert_eq!(forms.len(), 2, "{key}: expected one and other forms");
                let form = |forms: &serde_json::Map<String, Value>, category: &str, name: &str| {
                    forms
                        .get(category)
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| panic!("{key}: missing {name} {category} form"))
                        .to_owned()
                };
                let mut arms = String::new();
                for translation in &translations {
                    let translated = translation
                        .catalog
                        .get(key)
                        .unwrap_or(message)
                        .as_object()
                        .expect("matching plural message types");
                    let one = form(translated, "one", translation.variant);
                    let other = form(translated, "other", translation.variant);
                    let variant = translation.variant;
                    let body = plural(&one, &other, translation.singular);
                    write!(arms, "\n            {LANGUAGE}::{variant} => {body},").unwrap();
                }
                let english = plural(&form(forms, "one", "English"), &form(forms, "other", "English"), true);
                writeln!(
                    code,
                    "    ({key:?}, count = $count:expr $(, $($args:tt)*)?) => {{{{\n        let count = $count;\n        match $crate::i18n::current_language() {{{arms}\n            {LANGUAGE}::English => {english},\n        }}\n    }}}};"
                )
                .unwrap();
            }
            _ => panic!("{key}: expected a string or plural forms"),
        }
    }
    code.push_str("}\n");
    // Prefer English source names when translated aliases overlap. Duplicate
    // labels share an action name; custom labels absent from this list pass through.
    let mut aliases = BTreeMap::new();
    for &key in HISTORY_LABEL_KEYS {
        let source = messages
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{key}: expected a history label"));
        assert!(!source.contains('{'), "{key}: history labels cannot have format arguments");
        aliases.entry(source.to_owned()).or_insert(key);
        aliases.entry(key.to_owned()).or_insert(key);
    }
    for translation in &translations {
        for &key in HISTORY_LABEL_KEYS {
            let label = translation.catalog
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{key}: missing translated history label"));
            aliases.entry(label.to_owned()).or_insert(key);
        }
    }
    code.push_str("\npub(crate) fn history_label(label: &str) -> &str {\n    match label {\n");
    for (label, key) in aliases {
        writeln!(code, "        {label:?} => tr!({key:?}),").unwrap();
    }
    code.push_str("        _ => label,\n    }\n}\n");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(output.join("messages.rs"), code).expect("write the generated messages");
}
