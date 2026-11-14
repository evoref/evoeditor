//! UI の言語と、その選択順 (docs/07 §17.1・§17.4、05 §6.1)。
//!
//! 選択順は `--locale` > `ui.locale` > 環境変数 `EVE_LANG` > OS ロケール > 既定の `ja`。各段の値の言語の部分 (地域などを
//! 落とす。`fr-FR` → `fr`) が、同梱の言語 (`ja` / `en`) か、ユーザーの言語ファイルの候補 (`<config_dir>/locales/` を bin が
//! 一覧して作る集合、07 §17.4) にある言語に当たれば、その言語を選ぶ。選択では中身を読まない。
//! 利用者が書いた値 (`--locale`・`EVE_LANG`) が使える言語に当たらないときは警告して次の段へ進む (bin が
//! `msg.config.locale_not_found` の診断にする)。`ui.locale` の値の形の誤りは設定の読込 (`load_str`) が、使える言語との照合は
//! [`check_ui_locale`](crate::check_ui_locale) が診断にするので、ここでは再び警告しない。OS ロケールは「候補」にすぎないので、
//! 使える言語に当たらなくても黙って次の段へ進む。

use std::fmt;

/// UI の言語。言語の部分の名前 (ISO 639 の 2〜3 字の英小文字。`ja`・`en`・`fr`) を持ち、補う同梱の言語
/// ([`Locale::base`]。同梱の言語ならその言語、同梱していない言語 (追加言語) なら `en`。07 §17.4 の「重ね方」) が決まる。
/// 名前は 3 バイトに収まるので `Copy` にする。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Locale {
    /// 言語の部分の名前 (ASCII の英小文字。`len` を超える部分は 0)。
    name: [u8; 3],
    /// 名前の長さ (2 か 3)。
    len: u8,
}

/// Windows の予約のデバイス名のうち、言語ファイルの名前の形 (英小文字 2〜3 字) に当たるもの (07 §17.4)。
const DEVICE_NAMES: [&str; 4] = ["con", "aux", "prn", "nul"];

#[allow(non_upper_case_globals)]
impl Locale {
    /// 日本語 (既定。同梱)。
    pub const Ja: Locale = Locale {
        name: *b"ja\0",
        len: 2,
    };
    /// 英語 (同梱)。
    pub const En: Locale = Locale {
        name: *b"en\0",
        len: 2,
    };
    /// 同梱の言語 (`ja`・`en` の順)。
    pub const BUILTIN: [Locale; 2] = [Locale::Ja, Locale::En];

    /// 言語の部分の名前 (英小文字 2〜3 字) から作る。形に合わなければ `None`。
    fn from_lang(lang: &str) -> Option<Locale> {
        let b = lang.as_bytes();
        if !(2..=3).contains(&b.len()) || !b.iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        let mut name = [0u8; 3];
        for (dst, src) in name.iter_mut().zip(b) {
            *dst = src.to_ascii_lowercase();
        }
        Some(Locale {
            name,
            len: u8::try_from(b.len()).unwrap_or(3),
        })
    }

    /// 言語の名前 (`ja`・`fr` など。言語の部分だけの英小文字)。
    pub fn code(&self) -> &str {
        // 名前は ASCII の英小文字だけ (`from_lang` で作る) なので、いつも UTF-8 として読める
        std::str::from_utf8(&self.name[..usize::from(self.len)]).unwrap_or("")
    }

    /// 補う同梱の言語 (07 §17.4 の「重ね方」): 同梱の言語ならその言語、同梱していない言語 (追加言語) なら `en`。
    pub fn base(self) -> Locale {
        if self == Locale::Ja {
            Locale::Ja
        } else {
            Locale::En
        }
    }

    /// 同梱の言語 (`ja` / `en`) か。
    pub fn is_builtin(self) -> bool {
        Locale::BUILTIN.contains(&self)
    }

    /// 言語タグを **同梱の言語** に当てる。大文字小文字を区別せず、`_` は `-` と同じに扱い、地域などの後続を落とす
    /// (`ja-JP` → `ja`、`en_US` → `en`)。当たらなければ `None` (`auto`・空・同梱していない言語もここでは `None`)。
    pub fn parse(tag: &str) -> Option<Locale> {
        Locale::from_tag(tag).filter(|l| l.is_builtin())
    }

    /// 言語タグ (07 §17.4 の「値の形」: `^[A-Za-z]{2,3}([-_][A-Za-z0-9]{1,8})*$`。前後の空白と、POSIX のロケール名の
    /// `.codeset` / `@modifier` は除く) から、言語の部分の
    /// 言語を作る (`fr-FR` → `fr`。大文字小文字を区別しない)。形に合わない値 (`auto`・空を含む) は `None`。
    /// 使える言語に当たるか (同梱の言語か、候補にあるか) はここでは見ない。
    pub fn from_tag(tag: &str) -> Option<Locale> {
        // POSIX のロケール名の `.codeset` と `@modifier` は落としてから形を見る (`en_US.UTF-8`・`de_DE@euro`。
        // eve-platform の `tag_from_posix_env` と同じ)
        let tag = tag.trim();
        let tag = tag.split(['.', '@']).next().unwrap_or(tag);
        let mut parts = tag.split(['-', '_']);
        let lang = parts.next()?;
        let rest_ok = parts
            .all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()));
        if !rest_ok {
            return None;
        }
        Locale::from_lang(lang)
    }

    /// `locales/` の項目の名前が、ユーザーの言語ファイルの名前の規則 (07 §17.4: `^[a-z]{2,3}\.toml$` に完全に一致し、
    /// Windows のデバイス名 `con` / `aux` / `prn` / `nul` でない) に合えば、その言語。大文字・地域付き (`FR.toml`・
    /// `zh-TW.toml`・`pt_BR.toml`) は `None`。名前は一覧で得た綴りのまま渡す (大文字小文字を区別しない FS でも、綴りで照合する)。
    pub fn from_file_name(name: &str) -> Option<Locale> {
        let stem = name.strip_suffix(".toml")?;
        if !(2..=3).contains(&stem.len())
            || !stem.bytes().all(|b| b.is_ascii_lowercase())
            || DEVICE_NAMES.contains(&stem)
        {
            return None;
        }
        Locale::from_lang(stem)
    }
}

impl Default for Locale {
    fn default() -> Locale {
        Locale::Ja
    }
}

impl fmt::Debug for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Locale({})", self.code())
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// 選択順の段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocaleSource {
    /// コマンドラインの `--locale`。
    Cli,
    /// 設定ファイルの `ui.locale`。
    Config,
    /// 環境変数 `EVE_LANG`。
    Env,
    /// OS のロケール。
    Os,
    /// どれにも当たらず、既定の `ja`。
    Default,
}

/// 使える言語に当たらなかった、利用者が書いた値 (`msg.config.locale_not_found` の診断とログに出す)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleWarning {
    /// `Cli` か `Env`。
    pub source: LocaleSource,
    /// 利用者が書いた値そのまま (表示の前に無害化する)。
    pub value: String,
}

/// 選択の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleChoice {
    /// 選ばれた言語。
    pub locale: Locale,
    /// 選んだ段。
    pub source: LocaleSource,
    /// 使える言語に当たらなかった値 (選択順に。診断とログに出す)。
    pub warnings: Vec<LocaleWarning>,
}

/// 値が「未指定」か (空・`auto`)。
fn is_auto(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t.eq_ignore_ascii_case("auto")
}

/// 言語を選ぶ。`config` は `ui.locale` を読んだ結果 (`auto` または形の誤りは `None`)、`available` はユーザーの言語ファイルの
/// 候補の言語 (bin が `locales/` を一覧して作る。`--no-config` では空)。同梱の言語はいつも使える。
pub fn select_locale(
    cli: Option<&str>,
    config: Option<Locale>,
    env: Option<&str>,
    os: Option<&str>,
    available: &[Locale],
) -> LocaleChoice {
    let usable = |l: &Locale| l.is_builtin() || available.contains(l);
    let lookup = |v: &str| Locale::from_tag(v).filter(usable);
    let mut warnings = Vec::new();
    let mut user_value = |source: LocaleSource, value: Option<&str>| -> Option<Locale> {
        let v = value.filter(|v| !is_auto(v))?;
        let found = lookup(v);
        if found.is_none() {
            warnings.push(LocaleWarning {
                source,
                value: v.to_owned(),
            });
        }
        found
    };
    let from_cli = user_value(LocaleSource::Cli, cli);
    let from_config = config.filter(usable);
    let from_env = if from_cli.is_none() && from_config.is_none() {
        user_value(LocaleSource::Env, env)
    } else {
        None
    };
    let (locale, source) = if let Some(l) = from_cli {
        (l, LocaleSource::Cli)
    } else if let Some(l) = from_config {
        (l, LocaleSource::Config)
    } else if let Some(l) = from_env {
        (l, LocaleSource::Env)
    } else if let Some(l) = os.and_then(lookup) {
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

    fn fr() -> Locale {
        Locale::from_tag("fr").unwrap()
    }

    // NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
    // verifies: NFR-603
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

    // verifies: NFR-603
    #[test]
    fn 言語タグの形と言語の部分() {
        for (tag, want) in [
            ("fr", Some("fr")),
            ("fr-FR", Some("fr")),
            ("ZH_hant_TW", Some("zh")),
            ("kab", Some("kab")),
            ("en-a1b2c3d4", Some("en")),
            ("auto", None),
            ("", None),
            ("f", None),
            ("fren", None),
            ("fr-", None),
            ("fr-abcdefghi", None),
            ("f1", None),
            ("../x", None),
            ("fr/..", None),
            ("ｆｒ", None),
            ("en_US.UTF-8", Some("en")),
            ("ja_JP.eucJP@x", Some("ja")),
            ("de_DE@euro", Some("de")),
            (".UTF-8", None),
        ] {
            assert_eq!(
                Locale::from_tag(tag)
                    .map(|l| l.code().to_owned())
                    .as_deref(),
                want,
                "{tag}"
            );
        }
        assert_eq!(fr().base(), Locale::En, "追加言語は en で補う");
        assert_eq!(Locale::Ja.base(), Locale::Ja);
        assert_eq!(Locale::En.base(), Locale::En);
        assert!(!fr().is_builtin());
        assert_eq!(format!("{:?} {}", fr(), fr()), "Locale(fr) fr");
        assert_eq!(Locale::default(), Locale::Ja);
    }

    // verifies: NFR-603
    #[test]
    fn 言語ファイルの名前の規則() {
        for (name, want) in [
            ("fr.toml", Some("fr")),
            ("kab.toml", Some("kab")),
            ("ja.toml", Some("ja")),
            ("FR.toml", None),
            ("Fr.toml", None),
            ("fr.TOML", None),
            ("zh-TW.toml", None),
            ("pt_BR.toml", None),
            ("f.toml", None),
            ("fren.toml", None),
            ("con.toml", None),
            ("aux.toml", None),
            ("prn.toml", None),
            ("nul.toml", None),
            ("fr.toml.bak", None),
            ("fr", None),
            (".toml", None),
        ] {
            assert_eq!(
                Locale::from_file_name(name)
                    .map(|l| l.code().to_owned())
                    .as_deref(),
                want,
                "{name}"
            );
        }
    }

    // NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
    #[test]
    fn 選択順は_cli_設定_環境変数_os_既定() {
        let all = select_locale(Some("en"), Some(Locale::Ja), Some("ja"), Some("ja-JP"), &[]);
        assert_eq!((all.locale, all.source), (Locale::En, LocaleSource::Cli));
        let c = select_locale(None, Some(Locale::En), Some("ja"), Some("ja-JP"), &[]);
        assert_eq!((c.locale, c.source), (Locale::En, LocaleSource::Config));
        let e = select_locale(None, None, Some("en_US"), Some("ja-JP"), &[]);
        assert_eq!((e.locale, e.source), (Locale::En, LocaleSource::Env));
        let o = select_locale(None, None, None, Some("en-GB"), &[]);
        assert_eq!((o.locale, o.source), (Locale::En, LocaleSource::Os));
        let d = select_locale(None, None, None, None, &[]);
        assert_eq!((d.locale, d.source), (Locale::Ja, LocaleSource::Default));
    }

    // NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
    // verifies: NFR-603
    #[test]
    fn 未知の言語名は警告して次の段へ進む() {
        let c = select_locale(Some("fr"), None, Some("en"), Some("ja-JP"), &[]);
        assert_eq!((c.locale, c.source), (Locale::En, LocaleSource::Env));
        assert_eq!(
            c.warnings,
            vec![LocaleWarning {
                source: LocaleSource::Cli,
                value: "fr".into()
            }]
        );
        let e = select_locale(None, None, Some("de_DE"), Some("en-US"), &[]);
        assert_eq!((e.locale, e.source), (Locale::En, LocaleSource::Os));
        assert_eq!(e.warnings.len(), 1);
        assert_eq!(e.warnings[0].source, LocaleSource::Env);
        // 形に合わない値も見つからない言語として警告する (パスを組み立てない)
        let p = select_locale(Some("../fr"), None, None, None, &[fr()]);
        assert_eq!(p.locale, Locale::Ja);
        assert_eq!(p.warnings[0].value, "../fr");
    }

    #[test]
    fn auto_と空は未指定として警告しない() {
        let c = select_locale(Some("auto"), None, Some(""), Some("fr-FR"), &[]);
        assert_eq!((c.locale, c.source), (Locale::Ja, LocaleSource::Default));
        assert!(c.warnings.is_empty(), "OS の候補が未対応でも警告しない");
    }

    #[test]
    fn 設定が決まっていれば環境変数は読まず警告もしない() {
        let c = select_locale(None, Some(Locale::Ja), Some("xx"), None, &[]);
        assert_eq!(c.source, LocaleSource::Config);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    // verifies: NFR-603
    #[test]
    fn どの段でも候補の言語を選べる() {
        let av = [fr()];
        let c = select_locale(Some("fr-FR"), None, None, None, &av);
        assert_eq!((c.locale, c.source), (fr(), LocaleSource::Cli));
        let s = select_locale(None, Some(fr()), None, None, &av);
        assert_eq!((s.locale, s.source), (fr(), LocaleSource::Config));
        let e = select_locale(None, None, Some("fr"), None, &av);
        assert_eq!((e.locale, e.source), (fr(), LocaleSource::Env));
        let o = select_locale(None, None, None, Some("fr_CA"), &av);
        assert_eq!((o.locale, o.source), (fr(), LocaleSource::Os));
        assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    }

    // verifies: NFR-603
    #[test]
    fn 候補に無い設定の言語は黙って次の段へ進む() {
        // `ui.locale` の照合の診断は `check_ui_locale` が出すので、ここでは警告しない
        let c = select_locale(None, Some(fr()), Some("en"), None, &[]);
        assert_eq!((c.locale, c.source), (Locale::En, LocaleSource::Env));
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }
}
