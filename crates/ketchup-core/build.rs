use serde_json::Value;
use std::{collections::BTreeMap, env, fmt::Write, fs, path::PathBuf};

fn main() {
    const CATALOG: &str = "../../assets/i18n/en.json";
    println!("cargo:rerun-if-changed={CATALOG}");
    let catalog = fs::read_to_string(CATALOG).expect("read the English message catalog");
    let messages: BTreeMap<String, Value> =
        serde_json::from_str(&catalog).expect("parse the English message catalog");

    // Keep static labels usable in constants and let Rust check formatted arguments.
    let mut code = String::from(
        "#[doc = \"Look up an application message in the bundled English catalog.\"]\n\
         #[macro_export]\nmacro_rules! tr {\n",
    );
    for (key, message) in messages {
        match message {
            Value::String(text) => {
                writeln!(code, "    ({key:?}) => {{ {text:?} }};").unwrap();
                if text.contains('{') {
                    writeln!(
                        code,
                        "    ({key:?}, $($args:tt)+) => {{ ::std::format!({text:?}, $($args)+) }};",
                    )
                    .unwrap();
                }
            }
            Value::Object(forms) => {
                assert_eq!(forms.len(), 2, "{key}: expected one and other forms");
                let form = |category| {
                    forms
                        .get(category)
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| panic!("{key}: missing {category} form"))
                };
                let one = form("one");
                let other = form("other");
                writeln!(
                    code,
                    r#"    ({key:?}, count = $count:expr $(, $($args:tt)*)?) => {{{{
        let count = $count;
        if count as f64 == 1.0 {{
            ::std::format!({one:?}, count = count $(, $($args)*)?)
        }} else {{
            ::std::format!({other:?}, count = count $(, $($args)*)?)
        }}
    }}}};"#,
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
