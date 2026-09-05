use super::*;
use std::collections::BTreeSet;

fn placeholders(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    for tail in value.split('{').skip(1) {
        if let Some((index, _)) = tail.split_once('}') {
            if index.parse::<usize>().is_ok() {
                result.push(index.to_owned());
            }
        }
    }
    result.sort();
    result
}

#[test]
fn every_catalog_is_complete_and_preserves_placeholders_and_safe_text() {
    let source = catalog(Language::En);
    assert!(source.len() > 600);
    let keys: BTreeSet<_> = source.keys().collect();
    for language in Language::ALL {
        let translated = catalog(language);
        assert_eq!(keys, translated.keys().collect(), "{}", language.code());
        for (key, value) in translated {
            assert!(!value.trim().is_empty(), "{}: {key}", language.code());
            assert_eq!(
                placeholders(&source[key]),
                placeholders(value),
                "{}: {key}",
                language.code()
            );
            assert_eq!(
                crate::output::safe_text(value),
                *value,
                "{}: unsafe {key}",
                language.code()
            );
        }
    }
}

#[test]
fn locale_precedence_and_unsupported_locales_are_explicit() {
    let resolve = |pairs: &[(&str, &str)]| {
        detect(|name| pairs.iter().find(|p| p.0 == name).map(|p| p.1.to_owned()))
    };
    assert_eq!(resolve(&[("LANG", "it_IT.UTF-8")]), Language::It);
    assert_eq!(
        resolve(&[("LANG", "it_IT.UTF-8"), ("LC_MESSAGES", "ja_JP.UTF-8")]),
        Language::Ja
    );
    assert_eq!(
        resolve(&[("LC_ALL", "C.UTF-8"), ("LANG", "it_IT.UTF-8")]),
        Language::En
    );
    assert_eq!(
        resolve(&[("LC_ALL", "zz_ZZ"), ("LANG", "it_IT")]),
        Language::En
    );
    assert_eq!(
        resolve(&[("LC_ALL", ""), ("LANG", "pt_BR.UTF-8")]),
        Language::Pt
    );
    assert_eq!(
        resolve(&[("SPEEDTEST_LANGUAGE", "AUTO"), ("LANG", "de_DE")]),
        Language::De
    );
    assert_eq!(
        resolve(&[("SPEEDTEST_LANGUAGE", " fr "), ("LC_ALL", "it_IT")]),
        Language::Fr
    );
    assert_eq!(
        resolve(&[("SPEEDTEST_LANGUAGE", "unsupported"), ("LANG", "it_IT")]),
        Language::En
    );
    for locale in ["zh_TW.UTF-8", "zh-Hant", "zh-HK", "zh_MO"] {
        assert_eq!(system_locale(locale), Language::En);
    }
    assert_eq!(system_locale("zh_CN.UTF-8"), Language::ZhCn);
    assert_eq!(system_locale("POSIX"), Language::En);
}

#[test]
fn explicit_language_before_or_after_command_and_separator_is_stable() {
    let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
    assert_eq!(
        from_arguments(&args(&["speedtest", "--language=it", "dns", "list"])),
        Language::It
    );
    assert_eq!(
        from_arguments(&args(&["speedtest", "dns", "list", "--language", "ja"])),
        Language::Ja
    );
    assert_eq!(
        from_arguments(&args(&[
            "speedtest",
            "--language",
            "fr",
            "--",
            "--language=ja"
        ])),
        Language::Fr
    );
    assert!(Language::parse("zh-TW").is_err());
}

#[test]
fn formatting_is_single_pass_and_preserves_data() {
    let key = "error: {0}";
    let data = "/tmp/{1}/CASE.json".to_string();
    let actual = message(Language::It, key, &[data.clone(), "replaced".into()]);
    assert!(actual.contains(&data));
    assert!(!actual.contains("replaced"));
    assert_eq!(
        text(Language::It, "  Download  ").trim(),
        text(Language::It, "Download")
    );
    assert_eq!(
        narrative(
            Language::Ja,
            "unknown vendor output https://host/CASE?q={0}"
        ),
        "unknown vendor output https://host/CASE?q={0}"
    );
}

#[test]
fn recognized_narrative_templates_preserve_numeric_evidence() {
    let key = "median latency rose from {0} ms idle to {1} ms under {2} load (+{3} ms, +{4}%).";
    let source = message(
        Language::En,
        key,
        &[
            "12.3".into(),
            "99.4".into(),
            "upload".into(),
            "87.1".into(),
            "708".into(),
        ],
    );
    for lang in Language::ALL
        .into_iter()
        .filter(|lang| *lang != Language::En)
    {
        let result = narrative(lang, &source);
        assert_ne!(result, source, "{}", lang.code());
        for number in ["12.3", "99.4", "87.1", "708"] {
            assert!(result.contains(number));
        }
    }
}

#[test]
fn render_scope_is_nested_thread_local_and_does_not_change_cli_language() {
    let before = cli_language();
    let _outer = scope(Language::It);
    assert_eq!(ui("Settings"), text(Language::It, "Settings"));
    {
        let _inner = scope(Language::Ja);
        assert_eq!(ui("Settings"), text(Language::Ja, "Settings"));
    }
    assert_eq!(ui("Settings"), text(Language::It, "Settings"));
    std::thread::spawn(|| {
        assert_eq!(ui("Settings"), text(cli_language(), "Settings"));
    })
    .join()
    .unwrap();
    assert_eq!(cli_language(), before);
}
