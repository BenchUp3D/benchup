use serde_json::Value;
use std::{collections::BTreeMap, env, fmt::Write, fs, path::PathBuf};

type Catalog = BTreeMap<String, Value>;

const LANGUAGE: &str = "$crate::i18n::Language";

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
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(output.join("messages.rs"), code).expect("write the generated messages");
}
