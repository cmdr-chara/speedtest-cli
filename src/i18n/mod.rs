//! Offline presentation localization. Canonical data and process arguments stay invariant.
mod narrative;
#[cfg(test)]
mod tests;

use std::{cell::Cell, collections::BTreeMap, ffi::OsString, sync::OnceLock};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    #[default]
    Auto,
    En,
    It,
    Es,
    Fr,
    De,
    Pt,
    ZhCn,
    Ja,
}

impl Language {
    pub const ALL: [Self; 8] = [
        Self::En,
        Self::It,
        Self::Es,
        Self::Fr,
        Self::De,
        Self::Pt,
        Self::ZhCn,
        Self::Ja,
    ];
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "en" => Ok(Self::En),
            "it" => Ok(Self::It),
            "es" => Ok(Self::Es),
            "fr" => Ok(Self::Fr),
            "de" => Ok(Self::De),
            "pt" | "pt-br" | "pt-pt" => Ok(Self::Pt),
            "zh-cn" | "zh" | "zh-hans" => Ok(Self::ZhCn),
            "ja" => Ok(Self::Ja),
            _ => Err("language must be auto, en, it, es, fr, de, pt, zh-CN or ja".into()),
        }
    }
    pub const fn code(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::En => "en",
            Self::It => "it",
            Self::Es => "es",
            Self::Fr => "fr",
            Self::De => "de",
            Self::Pt => "pt",
            Self::ZhCn => "zh-CN",
            Self::Ja => "ja",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::En => "English",
            Self::It => "Italiano",
            Self::Es => "Español",
            Self::Fr => "Français",
            Self::De => "Deutsch",
            Self::Pt => "Português",
            Self::ZhCn => "简体中文",
            Self::Ja => "日本語",
        }
    }
    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|x| *x == self).unwrap_or(0);
        Self::ALL[(index + if forward { 1 } else { 7 }) % 8]
    }
    pub fn resolve(self) -> Self {
        if self != Self::Auto {
            self
        } else {
            detect(|name| std::env::var(name).ok())
        }
    }
}

/// Locale precedence is deterministic and testable without mutating process state.
pub fn detect(mut get: impl FnMut(&str) -> Option<String>) -> Language {
    if let Some(value) = get("SPEEDTEST_LANGUAGE").filter(|v| !v.trim().is_empty()) {
        match Language::parse(value.trim()) {
            Ok(Language::Auto) => {}
            Ok(language) => return language,
            Err(_) => return Language::En,
        }
    }
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = get(name).filter(|v| !v.trim().is_empty()) {
            return system_locale(&value);
        }
    }
    Language::En
}
fn system_locale(value: &str) -> Language {
    let value = value
        .trim()
        .split('.')
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .replace('_', "-")
        .to_ascii_lowercase();
    if value.starts_with("zh-")
        && ["tw", "hk", "mo", "hant"]
            .iter()
            .any(|s| value.split('-').any(|p| p == *s))
    {
        return Language::En;
    }
    Language::parse(value.split('-').next().unwrap_or("")).unwrap_or(Language::En)
}

pub fn from_arguments(arguments: &[OsString]) -> Language {
    let mut language = Language::Auto;
    for (index, value) in arguments.iter().enumerate() {
        let value = value.to_str().unwrap_or("");
        if value == "--" {
            break;
        }
        let code = value.strip_prefix("--language=").or_else(|| {
            (value == "--language")
                .then(|| arguments.get(index + 1).and_then(|v| v.to_str()))
                .flatten()
        });
        if let Some(code) = code {
            language = Language::parse(code).unwrap_or(Language::En);
        }
    }
    language.resolve()
}
static CLI: OnceLock<Language> = OnceLock::new();
pub fn initialize_cli(language: Language) {
    let _ = CLI.set(language.resolve());
}
pub fn cli_language() -> Language {
    CLI.get().copied().unwrap_or(Language::En)
}

// A draw is synchronous. Scope is thread-local and restored on unwind, so parallel
// render tests and unrelated CLI tasks cannot change each other's presentation.
thread_local! { static UI: Cell<Option<Language>> = const { Cell::new(None) }; }
pub struct Scope(Option<Language>);
pub fn scope(language: Language) -> Scope {
    Scope(UI.replace(Some(language)))
}
impl Drop for Scope {
    fn drop(&mut self) {
        UI.set(self.0);
    }
}
pub fn ui(source: impl AsRef<str>) -> String {
    narrative(UI.get().unwrap_or_else(cli_language), source.as_ref())
}

type Catalog = BTreeMap<String, String>;
static CATALOGS: OnceLock<Vec<Catalog>> = OnceLock::new();
fn catalog(language: Language) -> &'static Catalog {
    let catalogs = CATALOGS.get_or_init(|| {
        [
            include_str!("locales/en.json"),
            include_str!("locales/it.json"),
            include_str!("locales/es.json"),
            include_str!("locales/fr.json"),
            include_str!("locales/de.json"),
            include_str!("locales/pt.json"),
            include_str!("locales/zh-CN.json"),
            include_str!("locales/ja.json"),
        ]
        .iter()
        .map(|s| serde_json::from_str(s).expect("embedded catalog validated by tests"))
        .collect()
    });
    &catalogs[Language::ALL
        .iter()
        .position(|l| *l == language)
        .unwrap_or(0)]
}
fn normalize(source: &str) -> String {
    source.trim().to_ascii_lowercase()
}
fn is_heading(source: &str) -> bool {
    source.chars().any(char::is_alphabetic)
        && source
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(|c| !c.is_lowercase())
}
pub fn text(language: Language, source: &str) -> String {
    if matches!(language, Language::En | Language::Auto) {
        return source.to_owned();
    }
    let Some(value) = catalog(language).get(&normalize(source)) else {
        return source.to_owned();
    };
    let prefix = &source[..source.len() - source.trim_start().len()];
    let suffix = &source[source.trim_end().len()..];
    let value = if is_heading(source) {
        value.to_uppercase()
    } else {
        value.clone()
    };
    format!("{prefix}{value}{suffix}")
}

/// Numbered placeholders are substituted once. Inserted braces remain plain data.
pub fn message(language: Language, key: &str, values: &[String]) -> String {
    let template = text(language, key);
    let mut result = String::with_capacity(template.len());
    let mut rest = template.as_str();
    while let Some(pos) = rest.find('{') {
        result.push_str(&rest[..pos]);
        rest = &rest[pos..];
        if let Some(end) = rest.find('}') {
            if let Ok(index) = rest[1..end].parse::<usize>() {
                if let Some(value) = values.get(index) {
                    result.push_str(value);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        result.push('{');
        rest = &rest[1..];
    }
    result.push_str(rest);
    result
}
pub fn narrative(language: Language, source: &str) -> String {
    narrative::translate(language, source)
}

/// Only owned help prose is translated. Command/argument IDs and value enums stay stable.
pub fn command(mut command: clap::Command, language: Language) -> clap::Command {
    if let Some(value) = command.get_about().map(|v| text(language, &v.to_string())) {
        command = command.about(value);
    }
    if let Some(value) = command
        .get_long_about()
        .map(|v| text(language, &v.to_string()))
    {
        command = command.long_about(value);
    }
    if let Some(value) = command
        .get_after_help()
        .map(|v| text(language, &v.to_string()))
    {
        command = command.after_help(value);
    }
    command
        .mut_args(|mut arg| {
            if let Some(value) = arg.get_help().map(|v| text(language, &v.to_string())) {
                arg = arg.help(value);
            }
            if let Some(value) = arg.get_long_help().map(|v| text(language, &v.to_string())) {
                arg = arg.long_help(value);
            }
            arg
        })
        .mut_subcommands(|cmd| self::command(cmd, language))
}
