//! 文書の言語と文書ごとの値 (docs/07 §9.6 の LANG-2 / LANG-6〜LANG-9、§3.6 の LAYER-1〜LAYER-5、01 FR-704)。
//!
//! - 言語定義の読込 (`ReadLanguages`、io のジョブ) を起動時と `config.reload` で積み、届いた定義の集まりに差し替える。
//! - 言語の判定: 読込の完了 (`put_document`)・新しい文書 (`add_document`)・名前を付けて保存でパスが変わったとき・
//!   ワークスペースのルートが決まったとき (明示の指定の無い文書)。`config.reload` では判定し直さない (LANG-7)。
//! - 文書ごとの値 (`editor.tab_width`・`editor.indent_unit`・`editor.indent_width`) を、言語と設定から決めて文書に写す。
//! - `lang.set` (LANG-8): 引数が無ければパレットの一覧で尋ねる。`auto` は明示の指定を外して判定し直す。
//!
//! spec: 07 §9.6, 07 §3.6

use eve_config::{DetectInput, Language, LanguageSet};
use eve_text::{ByteOff, ByteRange, TextRead};
use eve_view::{CutPolicy, DocumentId, InputFieldModel, ListItem, MessageLevel, OverlayModel};

use crate::command::ids;
use crate::command::{
    Args, CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};
use crate::document::{DisplaySettings, Document, IndentSettings, LoadState};
use crate::editor::state::EditorState;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobResult};
use crate::layout::cell_text;
use crate::ui_model::keys;
use crate::ui_model::overlay::Overlay;
use crate::ui_model::palette::{PaletteState, WINDOW, find, lower_graphemes};

/// `lang.set` の引数の値のうち、明示の指定を外して判定し直すもの (LANG-8)。
pub(crate) const LANG_AUTO: &str = eve_config::AUTO;

impl EditorState {
    /// 言語定義の集まり (組込みにユーザー層を重ねたもの)。
    pub fn languages(&self) -> &LanguageSet {
        &self.settings.languages
    }

    /// 文書の言語の定義 (`plain`・決めていない・定義の無い言語は `None`)。ルートの下の文書は、ワークスペース層の言語定義を
    /// 重ねた集まりから引く (07 §3.6 の LAYER-1 の (7))。
    pub(crate) fn language_of(&self, doc: &Document) -> Option<&Language> {
        doc.language
            .id
            .as_deref()
            .and_then(|id| self.language_set_for(doc).get(id))
    }

    /// 文書の判定と文書ごとの値に使う言語定義の集まり: ルートの下の文書でワークスペース層に言語定義があれば、それを重ねた
    /// 集まり。ほかは組込みにユーザー層を重ねた集まり。
    fn language_set_for(&self, doc: &Document) -> &LanguageSet {
        match &self.settings.workspace_language_set {
            Some(set) if doc.path.is_some() && self.doc_in_workspace(doc) => set,
            _ => &self.settings.languages,
        }
    }

    /// ワークスペース層 (07 §3.6 の LAYER-1 の (6)(7)) を効かせる文書か: ルートの下のパスの文書と、無題の文書 (無題は
    /// (6) だけ。言語は `plain`)。ルートが決まっていなければ無題の文書だけ。
    pub(crate) fn doc_in_workspace(&self, doc: &Document) -> bool {
        match &doc.path {
            None => doc.stdin.is_none(),
            Some(p) => self.workspace_relative(p).is_some(),
        }
    }

    /// 文書の言語 ID (`doc.language` の値。決めていなければ空)。
    pub(crate) fn language_id(doc: &Document) -> &str {
        doc.language.id.as_deref().unwrap_or("")
    }

    /// 文書ごとの値 (07 §3.6 の LAYER-1。段 5 では (1)〜(4)・(6)〜(8))。
    pub(crate) fn doc_values_of(&self, doc: &Document) -> eve_config::DocValues {
        eve_config::doc_values(&eve_config::DocLayers {
            config: &self.settings.config,
            origins: &self.settings.config_origins,
            user: self.settings.user_layer.as_ref(),
            language: self.language_of(doc),
            workspace: self.doc_in_workspace(doc),
        })
    }

    /// 文書に依らない、ワークスペース層を除いた文書ごとのキーの値 (07 §3.6 の (1)(2)(8)。既存のファイルの改行の判定に使う、
    /// LAYER-4)。
    pub(crate) fn user_doc_values(&self) -> eve_config::DocValues {
        eve_config::doc_values(&eve_config::DocLayers {
            config: &self.settings.config,
            origins: &self.settings.config_origins,
            user: self.settings.user_layer.as_ref(),
            language: None,
            workspace: false,
        })
    }

    /// 文書に写す表示の設定と字下げの設定 (文書ごとの値と、文書に依らない表示の設定から)。
    fn doc_settings(&self, doc: &Document) -> (DisplaySettings, IndentSettings) {
        let v = self.doc_values_of(doc);
        let c = &self.settings.config;
        (
            DisplaySettings {
                tab_width: v.tab_width.value,
                ambiguous_wide: c.ambiguous_width == 2,
                show_whitespace: c.show_whitespace == eve_config::ShowWhitespace::Minimal,
            },
            IndentSettings {
                unit: v.indent_unit.value,
                width: v.indent_width.value,
            },
        )
    }

    /// 文書 `id` の文書ごとの値を決め直して写す (本文は変えない。タブの幅が変われば、表示の単位と折り返しの覚えは鍵が変わる
    /// ので作り直される、LAYER-3)。
    pub(crate) fn refresh_doc_settings(&mut self, id: DocumentId) {
        let Some(d) = self.docs.documents.get(&id) else {
            return;
        };
        let (display, indent) = self.doc_settings(d);
        if let Some(d) = self.docs.documents.get_mut(&id)
            && (d.display != display || d.indent != indent)
        {
            d.display = display;
            d.indent = indent;
            self.out.needs_render = true;
        }
    }

    /// 開いている全文書の文書ごとの値を決め直す (設定・言語定義の差し替えの後)。
    pub(crate) fn refresh_all_doc_settings(&mut self) {
        let ids: Vec<DocumentId> = self.docs.documents.keys().copied().collect();
        for id in ids {
            self.refresh_doc_settings(id);
        }
    }

    /// 文書が揃った (新しい文書・読込の完了・先行表示) ときに呼ぶ: 言語を決めていなければ判定し、文書ごとの値を写す。
    /// 読込中の文書 (中身の無い仮の文書) は判定しない。
    pub(crate) fn settle_language(&mut self, id: DocumentId) {
        let undecided = self
            .docs
            .documents
            .get(&id)
            .is_some_and(|d| d.language.id.is_none() && d.load_state != LoadState::Loading);
        if undecided {
            self.detect_language(id);
        } else {
            self.refresh_doc_settings(id);
        }
    }

    /// 明示の指定が無ければ判定し直す (名前を付けて保存でパスが変わった、LANG-7)。
    pub(crate) fn redetect_language(&mut self, id: DocumentId) {
        let explicit = self
            .docs
            .documents
            .get(&id)
            .is_none_or(|d| d.language.explicit);
        if !explicit {
            self.detect_language(id);
        }
    }

    /// 明示の指定の無い、言語を決めた文書をすべて判定し直す (起動時の定義が文書より後に届いた・ワークスペースのルートが
    /// 決まった。起動時に開いた文書と後で開いた文書で言語が変わらないように、LANG-7)。
    pub(crate) fn redetect_languages(&mut self) {
        let ids: Vec<DocumentId> = self
            .docs
            .documents
            .iter()
            .filter(|(_, d)| !d.language.explicit && d.language.id.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.detect_language(id);
        }
    }

    /// 文書 `id` の言語を判定して写し、文書ごとの値を決め直す (07 §9.2 の 2〜4、LANG-6)。無題の文書は `plain`、標準入力の
    /// 文書は `first_line` だけで判定する。同じ層の 2 つの言語が当たったら、同じ組について 1 回だけ警告する。
    fn detect_language(&mut self, id: DocumentId) {
        let Some(d) = self.docs.documents.get(&id) else {
            return;
        };
        let name = d.path.as_ref().and_then(crate::FsPath::file_name);
        let rel = d.path.as_ref().and_then(|p| self.workspace_relative(p));
        let head = (d.path.is_some() || d.stdin.is_some()).then(|| doc_head(d));
        let first = head.as_deref().map(eve_config::first_line_of);
        let detected = self.language_set_for(d).detect(&DetectInput {
            file_name: name.as_deref(),
            rel_path: rel.as_deref(),
            first_line: first,
        });
        let lang = detected
            .language
            .map_or(eve_config::PLAIN, Language::name)
            .to_owned();
        let ambiguous = detected.ambiguous;
        if let Some(d) = self.docs.documents.get_mut(&id) {
            d.language.id = Some(lang);
        }
        self.refresh_doc_settings(id);
        if let Some(a) = ambiguous {
            self.note_ambiguous(a, name.as_deref().unwrap_or(""));
        }
    }

    /// `path` のワークスペースのルートからの相対パス (区切りは `/`)。ルートの外・ルートが決まっていなければ `None`。
    fn workspace_relative(&self, path: &crate::FsPath) -> Option<String> {
        let root = self.workspace()?.root();
        let rel = path.as_path().strip_prefix(root.as_path()).ok()?;
        let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
        Some(parts?.join("/"))
    }

    /// 同じ層の 2 つの言語が当たった警告 (`msg.language.ambiguous`、LANG-6 (d))。同じ組について 1 回だけ。場所は使う定義の、
    /// 当たった名前を書いた所。
    fn note_ambiguous(&mut self, a: eve_config::Ambiguity, file: &str) {
        if !self
            .settings
            .ambiguous_noted
            .insert((a.first.clone(), a.second.clone()))
        {
            return;
        }
        let eve_config::LangOrigin::File {
            source,
            line,
            column,
        } = a.origin
        else {
            return;
        };
        let d = eve_config::Diagnostic {
            severity: eve_config::Severity::Warning,
            kind: eve_config::DiagKind::Language(eve_config::LanguageProblem::Ambiguous {
                first: a.first,
                second: a.second,
            }),
            key: "language".to_owned(),
            value: file.to_owned(),
            line,
            column,
            condition: None,
        };
        self.report_config_more(&source, &[d]);
    }

    /// 言語定義の読込 (`ReadLanguages`) を積む。`startup` は起動時の 1 回 (届いたら、先に開き終わった文書を判定し直す)。
    /// 起動時は、Host が設定の読込元を渡したか、設定ファイルに言語定義があるときだけ積む (無ければ組込みの定義のまま)。
    pub(crate) fn start_languages_read(&mut self, startup: bool) {
        if startup {
            if self.settings.languages_requested {
                return;
            }
            self.settings.languages_requested = true;
            if self.io.config_sources.is_none() && self.settings.config_languages.is_empty() {
                return;
            }
        }
        self.spawn_languages_read(startup, false);
    }

    /// 起動の後に届いたワークスペース層の言語定義を重ねて読み直す (届いたら明示の指定の無い文書を判定し直し、診断はワークスペース
    /// 層のファイルの分だけを足す。ユーザー層の言語定義の診断は既に出しているので、2 重にしない)。
    pub(crate) fn spawn_languages_read_for_workspace(&mut self) {
        self.spawn_languages_read(true, true);
    }

    /// 言語定義の読込を積む。`redetect` なら、届いたときに明示の指定の無い文書の言語を判定し直す (起動時の読込と、起動の後に
    /// ワークスペース層の言語定義が届いたとき。LANG-7)。
    fn spawn_languages_read(&mut self, redetect: bool, workspace_only: bool) {
        let dir = self.config_sources().languages;
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::ReadLanguages {
                dir,
                config: self.settings.config_languages.clone(),
                workspace: self.settings.workspace_languages.clone(),
                // 起動の後のワークスペース層の契機は、届いたらすぐ判定し直すので、ジョブの中で正規表現まで組む (主スレッドで
                // 組まない)。ほかは `PrecompileLanguages` で組む (起動時の文書の読込を待たせない、07 §9.6 の段 2 の細目)
                precompile: workspace_only,
                workspace_files: self.settings.workspace_files.clone(),
            },
        );
        self.io.pending.insert(
            job,
            Pending::LanguagesRead {
                startup: redetect,
                workspace_only,
            },
        );
    }

    /// 届いた定義の集まりの `first_line` の正規表現を組むジョブを積む (io。起動時は起動時のパスを開くジョブの後ろになる)。
    /// ワークスペース層の集まりも組む (判定で初めて要ったときに主スレッドで組まない)。`workspace_only` なら、診断は
    /// ワークスペース層のファイルの分だけを足す。
    fn start_languages_precompile(&mut self, redetect: bool) {
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::PrecompileLanguages {
                set: self.settings.languages.clone(),
                workspace: self.settings.workspace_language_set.clone(),
                workspace_files: self.settings.workspace_files.clone(),
            },
        );
        self.io
            .pending
            .insert(job, Pending::LanguagesPrecompile { redetect });
    }

    /// 正規表現を組んだ結果: 組めなかった `first_line` の診断を出す (今の定義の集まりのものだけ)。`redetect` なら、組んだ後で
    /// 明示の指定の無い文書の言語を判定し直す (起動時の読込。判定で正規表現を主スレッドで組まない)。
    pub(crate) fn finish_languages_precompile(&mut self, r: JobResult, redetect: bool) {
        let JobResult::LanguagesPrecompiled {
            set,
            workspace,
            reports,
        } = r
        else {
            return;
        };
        let same_ws = match (&workspace, &self.settings.workspace_language_set) {
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !std::sync::Arc::ptr_eq(&set, &self.settings.languages) || !same_ws {
            return;
        }
        for (file, diags) in &reports {
            self.report_config_more(file, diags);
        }
        if redetect {
            self.redetect_languages();
        }
    }

    /// 言語定義の読込の結果: 定義の集まりを差し替え、診断を出し、開いている文書の文書ごとの値を決め直す。起動時の読込なら、
    /// 先に開き終わった文書 (標準入力など) の言語を判定し直す (LANG-7)。`config.reload` の読込では判定し直さない。
    pub(crate) fn finish_languages_read(
        &mut self,
        r: JobResult,
        startup: bool,
        workspace_only: bool,
    ) {
        let JobResult::LanguagesRead {
            set,
            workspace_set,
            reports,
        } = r
        else {
            return;
        };
        self.settings.languages = set;
        self.settings.workspace_language_set = workspace_set;
        self.settings.ambiguous_noted.clear();
        // 言語定義のファイルの診断は読み直した結果に置き換える。設定ファイルの言語定義の診断 (正規表現など) は、設定ファイルの
        // 診断に足す (設定ファイルの診断は、起動時の報告か `config.reload` が先に置き換えている)
        // ワークスペース層の言語定義が届いて読み直したときは、ワークスペース層のファイルの診断だけを足す (ほかは出したまま)
        let reports: Vec<_> = reports
            .into_iter()
            .filter(|(f, _)| !workspace_only || self.settings.workspace_files.contains(f))
            .collect();
        if !workspace_only {
            self.forget_config_diagnostics(|f| {
                f.starts_with(crate::io::languages::LANGUAGES_PREFIX)
            });
        }
        for (file, diags) in &reports {
            if file.starts_with(crate::io::languages::LANGUAGES_PREFIX) {
                self.report_config(file, diags);
            } else {
                self.report_config_more(file, diags);
            }
        }
        self.refresh_all_doc_settings();
        if workspace_only {
            // 正規表現はジョブの中で組んである
            if startup {
                self.redetect_languages();
            }
        } else {
            // 判定し直すのは正規表現を組んだ後 (主スレッドで組まない)
            self.start_languages_precompile(startup);
        }
    }

    /// 設定ファイルの言語定義を差し替える (起動時の Host と `config.reload`)。
    pub(crate) fn set_config_languages(&mut self, defs: Vec<eve_config::LanguageDef>) {
        self.settings.config_languages = defs;
    }

    /// 文書 `id` の言語を `lang` (定義のある言語 ID か `auto`) にする (LANG-8)。替えたら文書ごとの値を決め直し、
    /// `info.lang.changed` を出す。
    fn set_document_language(&mut self, id: DocumentId, lang: &str) -> CommandResult {
        if lang == LANG_AUTO {
            if let Some(d) = self.docs.documents.get_mut(&id) {
                d.language.explicit = false;
            }
            self.detect_language(id);
        } else {
            if lang != eve_config::PLAIN && self.settings.languages.get(lang).is_none() {
                return CommandResult::Rejected(RejectReason::NotAvailable);
            }
            if let Some(d) = self.docs.documents.get_mut(&id) {
                d.language = crate::document::DocLanguage {
                    id: Some(lang.to_owned()),
                    explicit: true,
                };
            }
            self.refresh_doc_settings(id);
        }
        let shown = self
            .docs
            .documents
            .get(&id)
            .map(|d| self.language_label(Self::language_id(d)))
            .unwrap_or_default();
        self.show_message(
            MessageLevel::Info,
            keys::INFO_LANG_CHANGED,
            &[("language", &shown)],
        );
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 言語の表示 (`display_name (ID)`。定義の無い ID はそのまま)。ファイル由来の文字列を含むので無害化する。
    fn language_label(&self, id: &str) -> String {
        let id = eve_config::sanitize_untrusted(id);
        match self.settings.languages.get(&id) {
            Some(l) if l.display_name() != id => {
                format!(
                    "{} ({id})",
                    eve_config::sanitize_untrusted(l.display_name())
                )
            }
            _ => id,
        }
    }

    /// `lang.set` の一覧の候補 (値, 表示, ID の列)。先頭は `auto`、続けて `plain` と定義のある言語 (組込みの表の順、ユーザー層で
    /// 足した順)。
    fn lang_set_choices(&self) -> Vec<(String, String, String)> {
        let mut out = vec![(
            LANG_AUTO.to_owned(),
            self.catalog()
                .render(keys::UI_LANG_SET_AUTO, &[], &|_| None),
            LANG_AUTO.to_owned(),
        )];
        out.push((
            eve_config::PLAIN.to_owned(),
            eve_config::PLAIN.to_owned(),
            eve_config::PLAIN.to_owned(),
        ));
        for l in self.settings.languages.languages() {
            out.push((
                l.name().to_owned(),
                eve_config::sanitize_untrusted(l.display_name()),
                eve_config::sanitize_untrusted(l.name()),
            ));
        }
        out
    }

    /// 入力に合う候補の番号 (表示または ID の部分一致、大小を区別しない)。空の入力は全件。
    pub(crate) fn lang_set_hits(&self, query: &str) -> Vec<usize> {
        let q = lower_graphemes(query.trim());
        self.lang_set_choices()
            .iter()
            .enumerate()
            .filter(|(_, (_, label, id))| {
                q.is_empty()
                    || find(&lower_graphemes(label), &q).is_some()
                    || find(&lower_graphemes(id), &q).is_some()
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// `ui.confirm`: 選んだ言語でパレットを閉じ、`lang.set` を実行する。
    pub(crate) fn lang_set_confirm(&mut self) -> CommandResult {
        let Some(Overlay::Palette(p)) = &self.ui.overlay else {
            return CommandResult::Done;
        };
        let hits = self.lang_set_hits(&p.text);
        let Some(&i) = hits.get(p.selected.min(hits.len().saturating_sub(1))) else {
            return CommandResult::Done;
        };
        let value = self.lang_set_choices().swap_remove(i).0;
        self.set_overlay(None);
        self.ui.deferred_command = Some((
            ids::LANG_SET.to_owned(),
            Args::new().with_str("language", &value),
        ));
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 一覧の表示 (パレットと同じ `OverlayModel::List`)。行: 表示の名前、右に ID (今の言語には印)。
    pub(crate) fn lang_set_model(&self, p: &PaletteState) -> OverlayModel {
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let current = self
            .active_document()
            .and_then(|d| self.docs.documents.get(&d))
            .map(|d| Self::language_id(d).to_owned())
            .unwrap_or_default();
        let choices = self.lang_set_choices();
        let hits = self.lang_set_hits(&p.text);
        let selected = p.selected.min(hits.len().saturating_sub(1));
        let first = if selected < WINDOW {
            0
        } else {
            selected + 1 - WINDOW
        };
        let items: Vec<ListItem> = hits
            .iter()
            .skip(first)
            .take(WINDOW)
            .map(|&i| {
                let (value, label, id) = &choices[i];
                let detail = if i > 0 && *value == current {
                    render(keys::UI_LANG_SET_CURRENT, &[("language", id)])
                } else {
                    id.clone()
                };
                ListItem {
                    label: cell_text(label, CutPolicy::End),
                    detail: Some(cell_text(&detail, CutPolicy::End)),
                    match_ranges: Vec::new(),
                    keys: Vec::new(),
                    disabled_reason: None,
                }
            })
            .collect();
        let cells = cell_text(&p.text, CutPolicy::End).spans;
        let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
        let selection = crate::ui_model::overlay::field_selection(p.all_selected, cells.len());
        let footer = render(
            keys::UI_PALETTE_FOOTER,
            &[
                ("matched", &hits.len().to_string()),
                ("total", &choices.len().to_string()),
            ],
        );
        let title = render(
            keys::UI_LANG_SET_TITLE,
            &[("language", &eve_config::sanitize_untrusted(&current))],
        );
        OverlayModel::List {
            title: cell_text(&title, CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection,
                placeholder: None,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(&render(keys::UI_PALETTE_NO_MATCH, &[]), CutPolicy::End),
        }
    }
}

/// 文書の本文の先頭 (`first_line` に当てる 1 KiB と少し。char の境界で切る)。
fn doc_head(d: &Document) -> String {
    let t = d.text();
    let mut end = t.len_bytes().min(eve_config::FIRST_LINE_BYTES);
    while end > 0 && !t.is_char_boundary(ByteOff(end)) {
        end -= 1;
    }
    t.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(end)))
}

/// `lang.set`: 引数 `language` (言語 ID か `auto`) で、フォーカスの文書の言語を替える。引数が無ければパレットの一覧で尋ねる。
/// 読み取り専用の文書でも使える (本文を変えない)。Undo の履歴に残らない。
fn lang_set(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some(doc) = ctx.state.active_document() else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    match ctx.args.str("language").map(str::to_owned) {
        Some(lang) => ctx.state.set_document_language(doc, &lang),
        None => {
            ctx.state.set_overlay(Some(Overlay::Palette(PaletteState {
                languages: true,
                ..PaletteState::default()
            })));
            ctx.state.mark_dirty_render();
            CommandResult::Done
        }
    }
}

/// `lang.set` (07 §11.6)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(ids::LANG_SET, CommandKind::Ui, Permission::Ui, "", lang_set);
}
