//! Host が Editor に渡すもの (docs/02 §5.4、docs/review/v0.3-findings.md の STR-B3)。Host は `EditorState` に直接触らず
//! (`state_mut()` を使わず)、起動の前の設定は `Editor::configure`、起動の後に Host が知った事実は `Editor::report` で渡す。

use super::Editor;
use crate::io::config_reload::ConfigSources;

/// 起動の前に Host が渡す設定 (`start` / `run` の前)。`None` の項目は変えない。渡した順に、その場で当てる。
#[derive(Default)]
pub struct StartupConfig {
    /// ログファイルの場所 (設定の問題の要約に出す)。
    pub log_path: Option<Option<String>>,
    /// OS のロケール由来の文字コードのヒント (`file.default_encoding = "system"`、05 §6)。
    pub system_encoding: Option<Option<crate::Encoding>>,
    /// 設定の値。
    pub config: Option<eve_config::Config>,
    /// テーマファイルから読んだテーマ (名前、読めなければ `None`。07 §8.6)。
    pub user_theme: Option<(String, Option<eve_config::theme::Theme>)>,
    /// 設定の読込元 (`config.reload` が同じ元を読み直す、07 §6)。
    pub config_sources: Option<ConfigSources>,
    /// UI の言語 (07 §17.1)。
    pub locale: Option<eve_config::Locale>,
}

/// 起動の後に Host が Editor に知らせる事実。
pub enum HostReport<'a> {
    /// frontend の編集面の大きさ (起動のとき。以後の大きさの変化は `InputEvent::Resize`)。
    Viewport { cols: u16, rows: u16 },
    /// 設定ファイルの読込の診断 (最初の Frame の後にメッセージとして出す、07 §4.3)。`file` は表示名。
    ConfigDiagnostics {
        file: &'a str,
        diagnostics: &'a [eve_config::Diagnostic],
    },
}

impl Editor {
    /// 起動の前の設定を当てる。項目の順は `StartupConfig` の欄の順 (Host の呼び出しの順と同じになるよう、
    /// Host は 1 項目ずつ渡す)。
    pub fn configure(&mut self, c: StartupConfig) {
        let st = &mut self.state;
        if let Some(path) = c.log_path {
            st.set_log_path(path);
        }
        if let Some(enc) = c.system_encoding {
            st.set_system_encoding(enc);
        }
        if let Some(config) = c.config {
            st.set_config(config);
        }
        if let Some((name, theme)) = c.user_theme {
            st.set_user_theme(&name, theme);
        }
        if let Some(sources) = c.config_sources {
            st.set_config_sources(sources);
        }
        if let Some(locale) = c.locale {
            st.set_locale(locale);
        }
    }

    /// Host が知った事実を渡す。
    pub fn report(&mut self, r: HostReport<'_>) {
        match r {
            HostReport::Viewport { cols, rows } => self.state.set_viewport(cols, rows),
            HostReport::ConfigDiagnostics { file, diagnostics } => {
                self.state.report_config(file, diagnostics);
            }
        }
    }
}
