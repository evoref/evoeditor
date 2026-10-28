//! UI の言語と、その選択順 (docs/07 §17.1、05 §6.1)。
//!
//! 選択順は `--locale` > `ui.locale` > 環境変数 `EVE_LANG` > OS ロケール > 既定の `ja`。
//! 利用者が書いた値 (`--locale`・`EVE_LANG`) が同梱の言語に当たらないときは警告して次の段へ進む。
//! `ui.locale` の不正な値は設定の読込 (`load_str`) が診断にするので、ここでは再び警告しない。
//! OS ロケールは「候補」にすぎないので、同梱の言語に当たらなくても黙って次の段へ進む。

/// 同梱している UI の言語。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    Ja,
    En,
}

impl Locale {
    /// 同梱の言語のコード (BCP 47 の言語部分)。
    pub fn code(self) -> &'static str {
        match self {
            Locale::Ja => "ja",
            Locale::En => "en",
        }
    }

    /// 言語タグを同梱の言語に当てる。大文字小文字を区別せず、`_` は `-` と同じに扱い、地域などの後続を落とす
    /// (`ja-JP` → `ja`、`en_US` → `en`)。当たらなければ `None` (`auto`・空もここでは `None`)。
    pub fn parse(tag: &str) -> Option<Locale> {
        let lang = tag.trim().split(['-', '_']).next()?;
        if lang.eq_ignore_ascii_case("ja") {
            Some(Locale::Ja)
        } else if lang.eq_ignore_ascii_case("en") {
            Some(Locale::En)
        } else {
            None
        }
    }
}

/// 選択順の段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocaleSource {
    Cli,
    Config,
    Env,
    Os,
    Default,
}

/// 同梱の言語に当たらなかった、利用者が書いた値 (ログに出す)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleWarning {
    /// `Cli` か `Env`。
    pub source: LocaleSource,
    pub value: String,
}

/// 選択の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleChoice {
    pub locale: Locale,
    pub source: LocaleSource,
    pub warnings: Vec<LocaleWarning>,
}

/// 値が「未指定」か (空・`auto`)。
fn is_auto(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t.eq_ignore_ascii_case("auto")
}

/// 言語を選ぶ。`config` は `ui.locale` を読んだ結果 (`auto` または不正な値は `None`)。
pub fn select_locale(
    cli: Option<&str>,
    config: Option<Locale>,
    env: Option<&str>,
    os: Option<&str>,
) -> LocaleChoice {
    let mut warnings = Vec::new();
    let mut user_value = |source: LocaleSource, value: Option<&str>| -> Option<Locale> {
        let v = value.filter(|v| !is_auto(v))?;
        let found = Locale::parse(v);
        if found.is_none() {
            warnings.push(LocaleWarning {
                source,
                value: v.to_owned(),
            });
        }
        found
    };
    let from_cli = user_value(LocaleSource::Cli, cli);
    let from_env = if from_cli.is_none() && config.is_none() {
        user_value(LocaleSource::Env, env)
    } else {
        None
    };
    let (locale, source) = if let Some(l) = from_cli {
        (l, LocaleSource::Cli)
    } else if let Some(l) = config {
        (l, LocaleSource::Config)
    } else if let Some(l) = from_env {
        (l, LocaleSource::Env)
    } else if let Some(l) = os.and_then(Locale::parse) {
        (l, LocaleSource::Os)
    } else {
        (Locale::Ja, LocaleSource::Default)
    };
    LocaleChoice {
        locale,
        source,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
    #[test]
    fn 言語タグは地域を落として当てる() {
        assert_eq!(Locale::parse("ja"), Some(Locale::Ja));
        assert_eq!(Locale::parse("ja-JP"), Some(Locale::Ja));
        assert_eq!(Locale::parse("JA_jp"), Some(Locale::Ja));
        assert_eq!(Locale::parse("en-US"), Some(Locale::En));
        assert_eq!(Locale::parse(" en "), Some(Locale::En));
        assert_eq!(Locale::parse("fr-FR"), None);
        assert_eq!(Locale::parse("jaz"), None, "前方一致はしない");
        assert_eq!(Locale::parse("auto"), None);
        assert_eq!(Locale::parse(""), None);
    }

    // NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
    #[test]
    fn 選択順は_cli_設定_環境変数_os_既定() {
        let all = select_locale(Some("en"), Some(Locale::Ja), Some("ja"), Some("ja-JP"));
        assert_eq!((all.locale, all.source), (Locale::En, LocaleSource::Cli));
        let c = select_locale(None, Some(Locale::En), Some("ja"), Some("ja-JP"));
        assert_eq!((c.locale, c.source), (Locale::En, LocaleSource::Config));
        let e = select_locale(None, None, Some("en_US"), Some("ja-JP"));
        assert_eq!((e.locale, e.source), (Locale::En, LocaleSource::Env));
        let o = select_locale(None, None, None, Some("en-GB"));
        assert_eq!((o.locale, o.source), (Locale::En, LocaleSource::Os));
        let d = select_locale(None, None, None, None);
        assert_eq!((d.locale, d.source), (Locale::Ja, LocaleSource::Default));
    }

    // NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
    #[test]
    fn 未知の言語名は警告して次の段へ進む() {
        let c = select_locale(Some("fr"), None, Some("en"), Some("ja-JP"));
        assert_eq!((c.locale, c.source), (Locale::En, LocaleSource::Env));
        assert_eq!(
            c.warnings,
            vec![LocaleWarning {
                source: LocaleSource::Cli,
                value: "fr".into()
            }]
        );
        let e = select_locale(None, None, Some("de_DE"), Some("en-US"));
        assert_eq!((e.locale, e.source), (Locale::En, LocaleSource::Os));
        assert_eq!(e.warnings.len(), 1);
        assert_eq!(e.warnings[0].source, LocaleSource::Env);
    }

    #[test]
    fn auto_と空は未指定として警告しない() {
        let c = select_locale(Some("auto"), None, Some(""), Some("fr-FR"));
        assert_eq!((c.locale, c.source), (Locale::Ja, LocaleSource::Default));
        assert!(c.warnings.is_empty(), "OS の候補が未対応でも警告しない");
    }

    #[test]
    fn 設定が決まっていれば環境変数は読まず警告もしない() {
        let c = select_locale(None, Some(Locale::Ja), Some("xx"), None);
        assert_eq!(c.source, LocaleSource::Config);
        assert!(c.warnings.is_empty());
    }
}
