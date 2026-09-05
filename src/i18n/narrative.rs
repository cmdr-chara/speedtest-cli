//! Translate recognized owned sentence templates only; never translate arbitrary
//! captured diagnostic reports, canonical fields, paths, or provider identifiers.
use super::{catalog, message, normalize, text, Language};

pub(super) fn translate(language: Language, source: &str) -> String {
    if matches!(language, Language::En | Language::Auto) {
        return source.to_owned();
    }
    if catalog(language).contains_key(&normalize(source)) {
        return text(language, source);
    }
    // Compile the closed set once; rendering must not sort the catalogs at each frame.
    static TEMPLATES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    let templates = TEMPLATES.get_or_init(|| {
        let mut templates: Vec<_> = catalog(Language::En)
            .values()
            .filter(|value| value.contains("{0}"))
            .map(String::as_str)
            .collect();
        templates.sort_by_key(|value| std::cmp::Reverse(value.find('{').unwrap_or(0)));
        templates
    });
    for template in templates {
        if let Some(values) = capture(template, source) {
            // Captures are data. Translate only the small vocabulary of embedded
            // domain enums, rather than feeding captures back through templates.
            let values = values
                .into_iter()
                .map(|v| {
                    if ["download", "upload", "high", "medium", "low", "on", "off"]
                        .contains(&v.as_str())
                    {
                        text(language, &v)
                    } else {
                        v
                    }
                })
                .collect::<Vec<_>>();
            let mut rendered = message(language, template, &values);
            if is_uppercase_template(source, template) {
                rendered = rendered.to_uppercase();
            }
            return rendered;
        }
    }
    source.to_owned()
}
fn is_uppercase_template(source: &str, template: &str) -> bool {
    super::is_heading(source) && !super::is_heading(template)
}

fn capture(template: &str, source: &str) -> Option<Vec<String>> {
    let mut rest = source;
    let mut pattern = template;
    let mut values = Vec::new();
    let mut literal_bytes = 0;
    while let Some(open) = pattern.find('{') {
        let prefix = &pattern[..open];
        literal_bytes += prefix.len();
        // ASCII case folding preserves byte boundaries, including CJK text.
        if !rest.get(..prefix.len())?.eq_ignore_ascii_case(prefix) {
            return None;
        }
        rest = &rest[prefix.len()..];
        let end = pattern[open..].find('}')? + open;
        let index = pattern[open + 1..end].parse::<usize>().ok()?;
        if index != values.len() {
            return None;
        }
        pattern = &pattern[end + 1..];
        let next = pattern.find('{').unwrap_or(pattern.len());
        let delimiter = &pattern[..next];
        let value_end = if delimiter.is_empty() {
            if !pattern.is_empty() {
                return None;
            }
            rest.len()
        } else {
            rest.to_ascii_lowercase()
                .find(&delimiter.to_ascii_lowercase())?
        };
        values.push(rest[..value_end].to_owned());
        rest = &rest[value_end..];
    }
    literal_bytes += pattern.len();
    if literal_bytes < 4 || !rest.eq_ignore_ascii_case(pattern) {
        return None;
    }
    Some(values)
}
