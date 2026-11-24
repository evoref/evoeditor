//! 設定の層の合成と出力 (docs/07 §3・§18.3): ユーザー層のファイル (`--config` で差し替え・複数可) を順に重ね、
//! 最後にコマンドライン層 (`--set key=value`) を重ねる。`--dump-config` の出力もここ。
//!
//! 起動時 (bin) と `config.reload` (core) が同じ規則で合成するよう、読み込んだ本文を受け取る純関数にしている。

use crate::config::{Config, declared_values, dump_rank, load_str_in};
use crate::{DiagKind, Diagnostic, Loaded, Severity};

/// ユーザー層のファイル 1 つ。`text` は、無ければ `Ok(None)`、読めなければ理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// 診断に出す名前 (`config.toml`、`--config` のパス)。
    pub name: String,
    /// 本文。ファイルが無ければ `Ok(None)`、読めなければ `Err` に理由。
    pub text: Result<Option<String>, String>,
}

/// 設定のキーを書いた所 (診断の場所に使う)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyOrigin {
    /// 書いた層の名前 (`config.toml`、`--config` のパス、`--set`)。
    pub source: String,
    /// 行 (1 始まり。`--set` では 0)。
    pub line: u32,
    /// 桁 (1 始まりの文字数。`--set` では 0)。
    pub column: u32,
    /// 条件付きセクション (07 §3.4) で書いた値なら、その条件。その値の後の診断 (テーマが無い・言語が無い) に添える。
    pub condition: Option<crate::Condition>,
}

/// 合成の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// 合成した設定。
    pub config: Config,
    /// (名前, 診断)。診断の無いものは含めない。
    pub reports: Vec<(String, Vec<Diagnostic>)>,
    /// エラー (重さ `Error`) の診断がある (`--check-config` の終了コード 3、`config.reload` の維持の判定)。
    pub has_error: bool,
    /// `theme.name` を最後に書いた所 (重ねた層のうち。どこにも書いていなければ `None`)。テーマが無い診断の場所に使う
    /// (07 §8.7)。
    pub theme_name_at: Option<KeyOrigin>,
    /// `ui.locale` を最後に書いた所と、書かれた値 (文字列でなければ空)。使える言語に当たらないときの診断
    /// ([`check_ui_locale`]) の場所と値に使う (07 §17.4)。
    pub ui_locale_at: Option<(KeyOrigin, String)>,
    /// 公開キーの値の由来 (層・ファイル・行・条件。`config.show`、07 §3.5)。
    pub origins: Origins,
    /// 設定ファイル (`config.toml`・`--config`) の `[[language]]` の定義 (重ねた順。`source` はファイルの表示名。07 §9.6 の
    /// LANG-2)。構文エラーのファイルのものは含めない。
    pub languages: Vec<crate::LanguageDef>,
    /// ユーザー層 (設定ファイルだけを重ねたもの)。ワークスペース層が届いたときに [`compose`] で重ね直す土台。
    pub user: UserLayer,
}

/// 公開キーの値の由来 (07 §3.5。`config.show` の右の列)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Origin {
    /// 既定値 (どの層も値を書いていない)。
    #[default]
    Default,
    /// ユーザー層のファイル (`config.toml`・`--config` のファイル)。
    User {
        /// 診断に出す名前と同じ表示名 (`config.toml`、`--config` のパス)。
        source: String,
        /// キーを書いた行 (1 始まり)。
        line: u32,
        /// キーを書いた桁 (1 始まり)。
        column: u32,
        /// 条件付きセクションの値なら、その条件 (07 §3.4 の COND-7)。
        condition: Option<crate::Condition>,
    },
    /// ワークスペース層 (`<root>/.eve/config.toml`、07 §12.5)。
    Workspace {
        /// ファイルの表示名 (`.eve/config.toml` の絶対パス。一覧の `Enter` で開く)。
        source: String,
        /// キーを書いた行 (1 始まり)。
        line: u32,
        /// キーを書いた桁 (1 始まり)。
        column: u32,
        /// 条件付きセクションの値なら、その条件。
        condition: Option<crate::Condition>,
    },
    /// コマンドライン層 (`--set`)。
    Cli {
        /// 条件付きのキー (`--set os.windows.editor.tab_width=2`) なら、その条件。
        condition: Option<crate::Condition>,
    },
}

impl Origin {
    /// 条件付きセクションの値なら、その条件。
    #[must_use]
    pub fn condition(&self) -> Option<crate::Condition> {
        match self {
            Origin::Default => None,
            Origin::User { condition, .. }
            | Origin::Workspace { condition, .. }
            | Origin::Cli { condition } => *condition,
        }
    }
}

/// 公開キーごとの値の由来。値を書いた層の無いキーは [`Origin::Default`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origins(std::collections::BTreeMap<&'static str, Origin>);

impl Origins {
    /// `key` の値の由来。
    #[must_use]
    pub fn of(&self, key: &str) -> &Origin {
        static DEFAULT: Origin = Origin::Default;
        self.0.get(key).unwrap_or(&DEFAULT)
    }

    fn set(&mut self, key: &'static str, origin: Origin) {
        self.0.insert(key, origin);
    }

    /// `key` の由来を `from` の `key` の由来にする (値を前の値に戻したとき、由来も揃える。設定の自動の再読込で Undo の
    /// 上限を下げる変化を効かせないとき、07 §6 の RLD-3)。`from` に無ければ既定 (由来の記録を外す)。
    pub fn copy_from(&mut self, key: &'static str, from: &Origins) {
        match from.0.get(key) {
            Some(o) => {
                self.0.insert(key, o.clone());
            }
            None => {
                self.0.remove(key);
            }
        }
    }
}

/// `ui.locale` を、使える言語 (同梱の言語と `available`。`available` は `locales/` を一覧して作る候補、07 §17.4) と
/// 照らす。`auto` でなく当たらなければ、`ui.locale` を書いた所に `msg.config.locale_not_found` (警告。`dir` は探した
/// `locales/` の絶対パスで、探していない (`--no-config` など) ときは空、`lang` は代わりに使う言語) を足す。値の形が合わない
/// 診断 (`load_str` が出した `msg.config.invalid_value`) は、許容値を使える言語の一覧に直す。どちらも、選択に使ったか
/// どうかに依らない。設定の値は変えない (使えない言語は [`select_locale`](crate::select_locale) が飛ばす)。
pub fn check_ui_locale(
    resolved: &mut Resolved,
    available: &[crate::Locale],
    dir: &str,
    lang: crate::Locale,
) {
    let allowed = locale_allowed(available);
    for (_, diags) in &mut resolved.reports {
        for d in diags.iter_mut().filter(|d| d.key == "ui.locale") {
            if let DiagKind::InvalidValue { allowed: a } = &mut d.kind {
                a.clone_from(&allowed);
            }
        }
    }
    let Some(l) = resolved.config.ui_locale else {
        return;
    };
    if l.is_builtin() || available.contains(&l) {
        return;
    }
    let (origin, value) = resolved.ui_locale_at.clone().unwrap_or_else(|| {
        (
            KeyOrigin {
                source: SET_SOURCE.to_owned(),
                line: 0,
                column: 0,
                condition: None,
            },
            l.code().to_owned(),
        )
    });
    let d = Diagnostic {
        severity: Severity::Warning,
        kind: DiagKind::Locale(crate::LocaleProblem::NotFound {
            dir: dir.to_owned(),
            lang: lang.code().to_owned(),
        }),
        key: "ui.locale".to_owned(),
        value,
        line: origin.line,
        column: origin.column,
        condition: origin.condition,
    };
    merge_reports(&mut resolved.reports, [(origin.source, vec![d])]);
}

/// `ui.locale` の許容値: `auto`・同梱の言語・候補の言語 (名前の順)。
fn locale_allowed(available: &[crate::Locale]) -> Vec<String> {
    let mut extra: Vec<&crate::Locale> = available.iter().filter(|l| !l.is_builtin()).collect();
    extra.sort();
    extra.dedup();
    ["auto", "ja", "en"]
        .into_iter()
        .map(str::to_owned)
        .chain(extra.into_iter().map(|l| l.code().to_owned()))
        .collect()
}

/// 診断の一覧に、同じ名前の層の診断を足す (無ければ後ろに足す)。テーマの診断を設定ファイルの名前で出すときに使う。
pub fn merge_reports(
    into: &mut Vec<(String, Vec<Diagnostic>)>,
    more: impl IntoIterator<Item = (String, Vec<Diagnostic>)>,
) {
    for (name, diags) in more {
        match into.iter_mut().find(|(n, _)| *n == name) {
            Some((_, ds)) => ds.extend(diags),
            None => into.push((name, diags)),
        }
    }
}

/// `--set` の診断に出す名前。
pub const SET_SOURCE: &str = "--set";

fn unreadable(message: String) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        kind: DiagKind::Syntax { message },
        key: String::new(),
        value: String::new(),
        line: 0,
        column: 0,
        condition: None,
    }
}

/// `--set key=value` を `base` に重ねる。値は TOML の値として読み、読めなければ文字列とする
/// (`--set view.wrap=soft` と書ける)。`=` が無ければ `None` (使い方の誤り)。条件付きのキー
/// (`--set os.windows.editor.tab_width=2`) は、`conditions` で条件が真のときだけ効く (07 §3.4 の COND-1)。
pub fn load_set(base: Config, arg: &str, conditions: crate::Conditions) -> Option<Loaded> {
    load_set_in(base, arg, conditions, None)
}

/// `load_set` と同じだが、ホームディレクトリ `home` を知っている (`resolve_home`)。
fn load_set_in(
    base: Config,
    arg: &str,
    conditions: crate::Conditions,
    home: Option<&str>,
) -> Option<Loaded> {
    let (key, value) = arg.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let value = value.trim();
    let as_toml = load_str_in(
        base.clone(),
        &format!("{key} = {value}\n"),
        conditions,
        home,
    );
    let syntax = as_toml
        .diagnostics
        .iter()
        .any(|d| matches!(d.kind, DiagKind::Syntax { .. } | DiagKind::Structure(_)));
    if !syntax {
        return Some(as_toml);
    }
    let quoted = value.replace('\\', "\\\\").replace('"', "\\\"");
    Some(load_str_in(
        base,
        &format!("{key} = \"{quoted}\"\n"),
        conditions,
        home,
    ))
}

/// ユーザー層 (設定ファイルを重ねたもの。ワークスペース層とコマンドライン層の下) の値と由来 (07 §3.1 の L0 + L2)。
/// ワークスペース層の「方向のみ」の比較の下の層 (07 §12.3) と、ワークスペース層を重ね直すときの土台にする。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserLayer {
    /// ユーザー層まで重ねた設定。
    pub config: Config,
    /// ユーザー層で値を書いた公開キーの由来。
    pub origins: Origins,
    /// `theme.name` を最後に書いた所。
    pub theme_name_at: Option<KeyOrigin>,
    /// `ui.locale` を最後に書いた所と、書かれた値。
    pub ui_locale_at: Option<(KeyOrigin, String)>,
}

/// ユーザー層にワークスペース層とコマンドライン層を重ねた結果 ([`compose`])。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed {
    /// 重ねた設定。
    pub config: Config,
    /// 公開キーの値の由来。
    pub origins: Origins,
    /// `theme.name` を最後に書いた所。
    pub theme_name_at: Option<KeyOrigin>,
    /// `ui.locale` を最後に書いた所と、書かれた値。
    pub ui_locale_at: Option<(KeyOrigin, String)>,
    /// `--set` の診断 (書いた順)。
    pub set_diagnostics: Vec<Diagnostic>,
    /// ワークスペース層の診断 (読込の診断と、ユーザー層と比べて使わなかった項目の診断。ワークスペース層が無ければ空)。
    pub workspace_diagnostics: Vec<Diagnostic>,
}

/// ユーザー層 `user` に、ワークスペース層 (`workspace` の (表示名, 層)。ユーザー層の値と比べて分類した安全な項目だけ、
/// 07 §12.3 / §12.5 の WSC-3) とコマンドライン層 (`sets`) をこの順に重ねる (07 §3.1 の L2 → L3 → L4)。`config.reload` と
/// ワークスペース層が届いたときに、設定ファイルを読み直さずに重ね直すのに使う。分類は重ねるたびに今の `user` で行う。
#[must_use]
pub fn compose(
    user: &UserLayer,
    workspace: Option<(&str, &crate::WorkspaceLayer)>,
    sets: &[String],
    conditions: crate::Conditions,
) -> Composed {
    compose_in(user, workspace, sets, conditions, None)
}

fn compose_in(
    user: &UserLayer,
    workspace: Option<(&str, &crate::WorkspaceLayer)>,
    sets: &[String],
    conditions: crate::Conditions,
    home: Option<&str>,
) -> Composed {
    let mut config = user.config.clone();
    let mut origins = user.origins.clone();
    let mut theme_name_at = user.theme_name_at.clone();
    let mut ui_locale_at = user.ui_locale_at.clone();
    let mut workspace_diagnostics = Vec::new();
    if let Some((source, ws)) = workspace {
        let classified = ws.classify(&user.config);
        ws.apply_to(&mut config, &classified.applied);
        workspace_diagnostics = classified.diagnostics;
        for w in &classified.applied {
            origins.set(
                w.key,
                Origin::Workspace {
                    source: source.to_owned(),
                    line: w.line,
                    column: w.column,
                    condition: w.condition,
                },
            );
        }
    }
    let mut set_diagnostics = Vec::new();
    for arg in sets {
        if let Some(l) = load_set_in(config.clone(), arg, conditions, home) {
            config = l.config;
            for w in &l.written {
                let at = KeyOrigin {
                    source: SET_SOURCE.to_owned(),
                    line: 0,
                    column: 0,
                    condition: w.condition,
                };
                match w.key {
                    "theme.name" => theme_name_at = Some(at),
                    "ui.locale" => ui_locale_at = Some((at, w.text.clone())),
                    _ => {}
                }
                origins.set(
                    w.key,
                    Origin::Cli {
                        condition: w.condition,
                    },
                );
            }
            set_diagnostics.extend(l.diagnostics);
        }
    }
    Composed {
        config,
        origins,
        theme_name_at,
        ui_locale_at,
        set_diagnostics,
        workspace_diagnostics,
    }
}

/// ユーザー層のファイルを順に重ね、最後に `sets` を重ねる。読めないファイル・構文エラーのファイルは重ねない
/// (既定値または前のファイルまでの値のまま。起動時はそれで動き、`config.reload` は `has_error` を見て前の設定を保つ)。
/// 条件付きセクションは、ファイルごと (`--set` は値ごと) に `conditions` で解いてから重ねる (07 §3.4 の COND-3。層の間の
/// 順は変えない)。ワークスペース層は含めない (起動の後のジョブで届き、[`compose`] で重ねる)。
pub fn resolve(sources: &[Source], sets: &[String], conditions: crate::Conditions) -> Resolved {
    resolve_home(sources, sets, conditions, None)
}

/// `resolve` と同じだが、ホームディレクトリ `home` を知っている: `workspace.safe_directories` にホームディレクトリそのものを
/// 書いたら、広すぎる場所の警告を書いた所に出す (07 §12.1 の OWN-5。起動時・`config.reload`・`--check-config` で同じ)。
pub fn resolve_home(
    sources: &[Source],
    sets: &[String],
    conditions: crate::Conditions,
    home: Option<&str>,
) -> Resolved {
    let mut user = UserLayer::default();
    let mut reports = Vec::new();
    let mut has_error = false;
    let mut languages = Vec::new();
    for s in sources {
        let loaded = match &s.text {
            Ok(None) => continue,
            Ok(Some(t)) => load_str_in(
                user.config.clone(),
                t.strip_prefix('\u{feff}').unwrap_or(t),
                conditions,
                home,
            ),
            Err(e) => Loaded {
                config: user.config.clone(),
                diagnostics: vec![unreadable(e.clone())],
                written: Vec::new(),
                languages: Vec::new(),
            },
        };
        let error = loaded
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error);
        has_error |= error;
        if !error {
            user.config = loaded.config;
            // 値を使った所を、由来と、テーマ・言語の診断の場所に記録する
            for w in &loaded.written {
                let at = KeyOrigin {
                    source: s.name.clone(),
                    line: w.line,
                    column: w.column,
                    condition: w.condition,
                };
                match w.key {
                    "theme.name" => user.theme_name_at = Some(at.clone()),
                    "ui.locale" => user.ui_locale_at = Some((at.clone(), w.text.clone())),
                    _ => {}
                }
                user.origins.set(
                    w.key,
                    Origin::User {
                        source: s.name.clone(),
                        line: w.line,
                        column: w.column,
                        condition: w.condition,
                    },
                );
            }
            languages.extend(loaded.languages.into_iter().map(|mut d| {
                d.source.clone_from(&s.name);
                d
            }));
        }
        if !loaded.diagnostics.is_empty() {
            reports.push((s.name.clone(), loaded.diagnostics));
        }
    }
    let composed = compose_in(&user, None, sets, conditions, home);
    has_error |= composed
        .set_diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);
    if !composed.set_diagnostics.is_empty() {
        reports.push((SET_SOURCE.to_owned(), composed.set_diagnostics));
    }
    Resolved {
        config: composed.config,
        reports,
        has_error,
        theme_name_at: composed.theme_name_at,
        ui_locale_at: composed.ui_locale_at,
        origins: composed.origins,
        languages,
        user,
    }
}

/// 公開キーの (キー, TOML の値の表記)。宣言の表 (07 §4.1) の順 (`config.show` の並び、§3.5)。
pub fn declared(c: &Config) -> Vec<(&'static str, String)> {
    declared_values(c)
}

/// 公開キーの (キー, TOML の値の表記)。並びはセクションごと (07 §5 の表の順) で、セクションの中は宣言の表の順。
/// 入れ子のキー (`file.swap.*`・`editor.undo.*`) は、そのセクションの直下のキーの後に置く。
pub fn values(c: &Config) -> Vec<(&'static str, String)> {
    let mut v = declared_values(c);
    v.sort_by_key(|(k, _)| dump_rank(k));
    v
}

/// `--dump-config` の出力。TOML ではセクションごとに並べ、JSON では平らなキーの表にする。
/// どちらも、そのまま設定ファイルとして (JSON は値の確認に) 使える形。
pub fn dump(c: &Config, json: bool) -> String {
    let vals = values(c);
    if json {
        let body: Vec<String> = vals
            .iter()
            .map(|(k, v)| format!("  \"{k}\": {v}"))
            .collect();
        return format!("{{\n{}\n}}\n", body.join(",\n"));
    }
    let mut out = String::new();
    let mut section = "";
    for (k, v) in &vals {
        let (sec, rest) = k.split_once('.').unwrap_or(("", k));
        if sec != section {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{sec}]\n"));
            section = sec;
        }
        out.push_str(&format!("{rest} = {v}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Osc52Policy, WrapMode, load_str};

    fn res(sources: &[Source], sets: &[String]) -> Resolved {
        resolve(sources, sets, crate::Conditions::default())
    }

    fn src(name: &str, text: &str) -> Source {
        Source {
            name: name.to_owned(),
            text: Ok(Some(text.to_owned())),
        }
    }

    #[test]
    fn theme_name_を最後に書いた所を覚える() {
        let at = |source: &str, line, column| {
            Some(KeyOrigin {
                source: source.to_owned(),
                line,
                column,
                condition: None,
            })
        };
        let a = src("a.toml", "[theme]\nname = \"x\"\n");
        let b = src("b.toml", "# c\n\ntheme.name = \"y\"\n");
        let none = src("c.toml", "[editor]\ntab_width = 3\n");
        assert_eq!(
            res(std::slice::from_ref(&a), &[]).theme_name_at,
            at("a.toml", 2, 1)
        );
        assert_eq!(
            res(&[a.clone(), b.clone(), none.clone()], &[]).theme_name_at,
            at("b.toml", 3, 7)
        );
        assert_eq!(
            res(&[b, a.clone()], &["theme.name=z".into()]).theme_name_at,
            at(SET_SOURCE, 0, 0)
        );
        assert_eq!(res(&[none], &[]).theme_name_at, None);
        // 構文エラーのファイルは重ねないので、書いた所にもしない
        let broken = src("d.toml", "[theme]\nname = \"w\"\n[editor\n");
        assert_eq!(res(&[a, broken], &[]).theme_name_at, at("a.toml", 2, 1));
    }

    // verifies: AC-NFR-603-9
    #[test]
    fn ui_locale_は使える言語と照らし_当たらなければ書いた所で知らせる() {
        let fr = crate::Locale::from_tag("fr").unwrap();
        let de = crate::Locale::from_tag("de").unwrap();
        let a = src("config.toml", "# c\n[ui]\nlocale = \"de-DE\"\n");
        // 使える言語 (同梱と候補) に当たらない: 書いた所と書いた値で、探した場所と代わりの言語を示す
        let mut r = res(std::slice::from_ref(&a), &[]);
        check_ui_locale(&mut r, &[fr], "/c/locales", crate::Locale::Ja);
        assert_eq!(r.config.ui_locale, Some(de), "設定の値は変えない");
        let (name, diags) = &r.reports[0];
        assert_eq!(name, "config.toml");
        assert_eq!(
            diags[0],
            Diagnostic {
                severity: Severity::Warning,
                kind: DiagKind::Locale(crate::LocaleProblem::NotFound {
                    dir: "/c/locales".to_owned(),
                    lang: "ja".to_owned(),
                }),
                key: "ui.locale".to_owned(),
                value: "de-DE".to_owned(),
                line: 3,
                column: 1,
                condition: None,
            }
        );
        // 候補にあれば知らせない。同梱の言語と auto も
        let mut ok = res(std::slice::from_ref(&a), &[]);
        check_ui_locale(&mut ok, &[de], "/c/locales", crate::Locale::Ja);
        assert!(ok.reports.is_empty(), "{:?}", ok.reports);
        for v in ["ja", "auto", "EN-us"] {
            let mut r = res(&[src("c.toml", &format!("ui.locale = \"{v}\"\n"))], &[]);
            check_ui_locale(&mut r, &[], "", crate::Locale::Ja);
            assert!(r.reports.is_empty(), "{v}: {:?}", r.reports);
        }
        // `--set` で書いたものは `--set` の診断
        let mut s = res(&[], &["ui.locale=xx".into()]);
        check_ui_locale(&mut s, &[], "", crate::Locale::Ja);
        assert_eq!(s.reports[0].0, SET_SOURCE);
        assert_eq!(s.reports[0].1[0].value, "xx");
        // 形の誤りの診断も、許容値を使える言語の一覧に直す
        let mut bad = res(&[src("c.toml", "ui.locale = \"../x\"\n")], &[]);
        check_ui_locale(&mut bad, &[fr, fr], "", crate::Locale::Ja);
        assert_eq!(bad.reports[0].1.len(), 1);
        assert!(matches!(
            &bad.reports[0].1[0].kind,
            DiagKind::InvalidValue { allowed } if allowed.last().map(String::as_str) == Some("fr")
                && allowed.len() == 4
        ));
    }

    #[test]
    fn 診断の一覧は同じ名前の層に足す() {
        let d = |key: &str| Diagnostic {
            severity: Severity::Warning,
            kind: DiagKind::Unsupported,
            key: key.to_owned(),
            value: String::new(),
            line: 1,
            column: 1,
            condition: None,
        };
        let mut r = vec![("config.toml".to_owned(), vec![d("a")])];
        merge_reports(
            &mut r,
            [
                ("config.toml".to_owned(), vec![d("b")]),
                ("themes/x.toml".to_owned(), vec![d("c")]),
            ],
        );
        assert_eq!(
            r,
            vec![
                ("config.toml".to_owned(), vec![d("a"), d("b")]),
                ("themes/x.toml".to_owned(), vec![d("c")]),
            ]
        );
    }

    #[test]
    fn ファイルを順に重ね_後のファイルが勝つ() {
        let r = res(
            &[
                src("a.toml", "[editor]\ntab_width = 8\nindent_width = 2\n"),
                src("b.toml", "[editor]\ntab_width = 3\n"),
            ],
            &[],
        );
        assert_eq!((r.config.tab_width, r.config.indent_width), (3, 2));
        assert!(!r.has_error && r.reports.is_empty());
    }

    #[test]
    fn 構文エラーのファイルは重ねず_エラーとして報告する() {
        let r = res(
            &[
                src("a.toml", "[editor]\ntab_width = 8\n"),
                src("b.toml", "[editor\n"),
            ],
            &[],
        );
        assert_eq!(r.config.tab_width, 8);
        assert!(r.has_error);
        assert_eq!(r.reports[0].0, "b.toml");
    }

    #[test]
    fn set_は最後に重ね_引用符なしの文字列も受け付ける() {
        let r = res(
            &[src("a.toml", "[view]\nwrap = \"word\"\n")],
            &["view.wrap=soft".into(), "editor.tab_width=2".into()],
        );
        assert_eq!(r.config.wrap, WrapMode::Soft);
        assert_eq!(r.config.tab_width, 2);
        let r = res(&[], &["editor.tab_width=99".into()]);
        assert_eq!(r.config.tab_width, 4, "範囲外は既定値");
        assert_eq!(r.reports[0].0, SET_SOURCE);
        assert!(load_set(Config::default(), "novalue", crate::Conditions::default()).is_none());
    }

    #[test]
    fn 出力は読み直すと同じ設定になる() {
        let c = Config {
            tab_width: 8,
            wrap: WrapMode::Word,
            ui_locale: Some(crate::Locale::En),
            osc52: Osc52Policy::Never,
            ..Config::default()
        };
        let l = load_str(&dump(&c, false));
        assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
        assert_eq!(l.config, c);
        // 公開キーをすべて出す
        assert_eq!(values(&c).len(), crate::config::KEYS.len());
        assert!(dump(&c, true).contains("\"editor.tab_width\": 8"));
    }

    #[test]
    fn 出力の並びはセクションごとで_入れ子のキーは後() {
        let keys: Vec<&str> = values(&Config::default())
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            keys,
            [
                "editor.tab_width",
                "editor.indent_unit",
                "editor.indent_width",
                "editor.backspace_unit",
                "editor.undo.total_max_bytes",
                "file.default_encoding",
                "file.default_line_ending",
                "file.huge_file_warn_bytes",
                "file.long_line_threshold",
                "file.reload_on_external_change",
                "file.swap.enabled",
                "file.swap.interval_ms",
                "file.swap.min_interval_ms",
                "file.swap.max_bytes",
                "file.swap.exclude_secret_files",
                "file.watch.enabled",
                "view.ambiguous_width",
                "view.wrap",
                "view.wrap_indent",
                "view.wrap_kinsoku",
                "view.show_whitespace",
                "view.line_numbers",
                "theme.color_mode",
                "theme.name",
                "theme.background",
                "ui.locale",
                "ui.mouse",
                "ui.hint_bar",
                "ui.tab_line",
                "ui.mouse_wheel_lines",
                "ui.which_key_delay_ms",
                "search.newline_sugar",
                "search.cp932_compat_fold",
                "search.case",
                "search.regex_default",
                "search.max_matches",
                "search.count_limit",
                "search.workspace.respect_gitignore",
                "search.workspace.follow_symlinks",
                "search.workspace.max_file_size",
                "clipboard.osc52",
                "clipboard.osc52_max_bytes",
                "workspace.root_markers",
                "workspace.exclude",
                "workspace.safe_directories",
            ]
        );
    }
}
