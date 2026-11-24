//! 設定ファイルの監視と自動の再読込 (FR-706、docs/07 §6 の RLD-1〜RLD-5、03 §9.5.1 の WATCH-12。v0.5 の M33 の段 9)。
//!
//! - **見るもの** (RLD-1): 読込元 (`ConfigSources`) の設定ファイル・`keymap.toml`・使っているテーマファイル (`inherits` の
//!   親も見るよう、`themes/` の `*.toml`)・`languages/*.toml`・ワークスペース層の `.eve/config.toml`・`.eve/keymap.toml`。
//!   置いたディレクトリを `io` のジョブ (`ResolveConfigWatch`) で canonical の形に解き (無いディレクトリは見ない。リンクで
//!   置いたファイルはリンクの先のディレクトリも見る)、文書の監視と同じ表 (`WatchState.dirs`) に載せる。同じディレクトリの
//!   文書と設定は 1 つの要求を共有する (02 §4.4 の「数え上げは呼び出し側が持つ」)。解き直すのは、監視を入れたとき・読込元を
//!   置いたとき・ワークスペースのルートが決まったとき・設定を読み直した後 (テーマの名前の変化・失効の張り直し)。
//! - **契機** (RLD-2): 見る名前の知らせ (と `Rescan`・`Lost`・Windows の短い名前) を、文書と同じ待ち合わせ (WATCH-4) で
//!   まとめ、`config.reload` と同じ手順で読み直す。自分の保存を抑えない。読み直しの最中 (ワークスペース層・テーマ・言語定義の
//!   読込の連鎖を含む) に期限が来たら、終わってから読む (契機を落とさない。期限は入れず、ジョブの結果で再開する)。
//! - **誤り** (RLD-3): 自動の読み直しのジョブは、読む前と読んだ後に大きさと更新日時を比べ (`verify`)、変わっていたら読んだ内容を
//!   捨てて待ち合わせからやり直す。`msg.config.reload_kept` は同じファイルの同じ内容で 1 回だけ。診断の要約は、自動の連鎖を
//!   始める前の一覧と同じなら出さない (段ごとではなく連鎖の前の一覧で比べる)。どの知らせも表示中の (読み直しのものでない)
//!   警告・エラーを置き換えない。`editor.undo.total_max_bytes` を小さくする変化は、値と由来を前のままにする。
//! - **止める** (RLD-4): `file.watch.enabled` だけで決める (`file.reload_on_external_change` に依らない)。Host が読込元を
//!   渡していないとき (試験の既定) は見ない。
//!
//! spec: 07 §6, 03 §9.5.1, 02 §4.4

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};

use eve_config::Diagnostic;
use eve_view::MessageLevel;

use crate::editor::state::EditorState;
use crate::io::fs::{FileSystem, FileTime};
use crate::io::fspath::FsPath;
use crate::io::pending::Pending;
use crate::io::watch::WatchError;
use crate::io::watch_doc::{DirState, DirWatch, Wait, maybe_short_name, name_key};
use crate::jobs::{JobKind, JobResult};
use crate::time::{Millis, Tick};
use crate::ui_model::config_report::Delivery;
use crate::ui_model::keys;

/// 設定のディレクトリを失効の後に張り直す最短の間隔 (続けて失効するディレクトリで張り直しと読み直しを繰り返さない)。
pub const CONFIG_RENEW_MIN: Millis = Millis(5000);

/// 監視の対象 (`ResolveConfigWatch` の入力)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigWatchTarget {
    /// このファイルを見る (無くても、置き場所のディレクトリがあれば作成を見る)。
    File(FsPath),
    /// このディレクトリの `*.toml` を見る。
    TomlDir(FsPath),
}

/// 解いた監視の対象 (`ResolveConfigWatch` の結果)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigWatchDir {
    /// 監視するディレクトリ (比べる形の canonical のパス、`same_file::canonical_key`)。
    pub dir: FsPath,
    /// 見る名前。`None` は `*.toml` の全部。
    pub name: Option<OsString>,
    /// 監視できないときの知らせ (`info.watch.unavailable` の `{path}`) に出すパス。
    pub shown: FsPath,
}

/// 対象を、監視するディレクトリと名前に解く (`io` のジョブの本体)。無いディレクトリは返さない。リンクで置いたファイルは、
/// 置いた所とリンクの先の両方を返す (リンクの先を書き換えても、リンクを差し替えても気付く)。
pub fn resolve_config_watch(
    fs: &dyn FileSystem,
    targets: &[ConfigWatchTarget],
) -> Vec<ConfigWatchDir> {
    let key = |p: &FsPath| FsPath::new(crate::io::same_file::canonical_key(p));
    let mut out: Vec<ConfigWatchDir> = Vec::new();
    let mut push = |d: ConfigWatchDir| {
        if !out.contains(&d) {
            out.push(d);
        }
    };
    for t in targets {
        match t {
            ConfigWatchTarget::File(p) => {
                let (Some(name), Some(parent)) = (p.as_path().file_name(), p.parent()) else {
                    continue;
                };
                if let Ok(dir) = fs.canonicalize(&parent) {
                    push(ConfigWatchDir {
                        dir: key(&dir),
                        name: Some(name.to_owned()),
                        shown: p.clone(),
                    });
                }
                if let Ok(c) = fs.canonicalize(p)
                    && let (Some(n), Some(d)) = (c.as_path().file_name(), c.parent())
                {
                    push(ConfigWatchDir {
                        dir: key(&d),
                        name: Some(n.to_owned()),
                        shown: p.clone(),
                    });
                }
            }
            ConfigWatchTarget::TomlDir(d) => {
                if let Ok(c) = fs.canonicalize(d) {
                    push(ConfigWatchDir {
                        dir: key(&c),
                        name: None,
                        shown: d.join("*.toml"),
                    });
                }
            }
        }
    }
    out
}

/// 読む前と読んだ後に比べるファイルの大きさと更新日時 (07 §6 の RLD-3)。無い・調べられないときは `None`。
pub(crate) type ReadStamp = Option<(u64, FileTime)>;

/// ファイルの大きさと更新日時。
pub(crate) fn read_stamp(fs: &dyn FileSystem, path: &FsPath) -> ReadStamp {
    fs.metadata(path).ok().map(|m| (m.len, m.mtime))
}

/// ディレクトリの `*.toml` の (名前, 大きさと更新日時) を名前の順に (言語定義の置き場所の確かめ、RLD-3)。
pub(crate) fn toml_stamps(fs: &dyn FileSystem, dir: &FsPath) -> Vec<(String, ReadStamp)> {
    let mut names: Vec<String> = fs
        .list_dir_names(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.ends_with(".toml"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let s = read_stamp(fs, &dir.join(&n));
            (n, s)
        })
        .collect()
}

/// 内容の要約 (同じファイルの同じ内容の `msg.config.reload_kept` を 1 回にする、RLD-3)。プロセスの中で比べるだけ。
pub(crate) fn digest(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// 1 つのディレクトリで見る設定ファイルの名前。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigNames {
    /// 見る名前 (照らす形、WATCH-3)。
    names: BTreeSet<OsString>,
    /// `*.toml` の全部を見る (`languages/`・`themes/`)。
    any_toml: bool,
    /// 監視できないときの知らせに出すパス。
    pub(crate) shown: FsPath,
}

impl ConfigNames {
    /// 知らせの名前が、見る設定ファイルか。
    fn matches(&self, name: &OsStr) -> bool {
        if self.names.contains(&name_key(name)) {
            return true;
        }
        self.any_toml && {
            let n = name.to_string_lossy();
            if cfg!(windows) {
                n.to_ascii_lowercase().ends_with(".toml")
            } else {
                n.ends_with(".toml")
            }
        }
    }
}

/// 自動の読み直しの 1 回の覚え (`apply_config_reload` が置き、キーマップを当てる `finish_reload_keymap` が使う)。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RunInfo {
    /// 自動の読み直し。
    pub(crate) auto: bool,
    /// `keymap.toml` の内容の要約 (読めたとき)。
    pub(crate) keymap_digest: Option<u64>,
}

/// 自動の読み直しの結果を当てている間の覚え。
#[derive(Clone, Debug)]
pub(crate) struct ReloadScope {
    /// 当てる前に、表示中のメッセージが警告・エラーだった (この回の知らせは履歴にだけ積む)。この回が出した警告は、この回の
    /// 後の知らせで置き換えてよい (同じ回の診断の要約の後の `reload_kept` を見せる)。
    alert: bool,
}

/// 設定の監視の状態 (`WatchState.config`)。
#[derive(Default)]
pub(crate) struct ConfigWatch {
    /// 設定を監視している (最後に当てた値)。
    on: bool,
    /// 解き直しの世代。
    generation: u64,
    /// 結果を待っている解き直し (世代と対象。同じ対象の解き直しを重ねて積まない)。
    resolving: Option<(u64, Vec<ConfigWatchTarget>)>,
    /// 待ち合わせ (WATCH-4 と同じ)。
    wait: Option<Wait>,
    /// 期限が来たときに読み直しの最中だった: 終わってから自動で読み直す。
    queued: bool,
    /// 自動の読み直しの最中に `config.reload` が来た: 終わってから手動で読み直す。
    queued_manual: bool,
    /// `msg.config.reload_kept` を出したファイル (表示名 → 内容の要約)。同じ内容では自動の読み直しで出し直さない。
    kept: BTreeMap<String, Option<u64>>,
    /// 自動の読み直しの結果を当てている間の覚え (`with_reload_scope`)。
    pub(crate) scope: Option<ReloadScope>,
    /// 自動の読み直しの連鎖を始める前の診断の一覧 (連鎖のどの段でも、これと同じ診断の要約は出し直さない)。連鎖が終わり、
    /// 待ち合わせ・順番待ちも無くなったら捨てる (読む間に変わって捨てた回の続きは、同じ一覧で比べる)。
    chain_diags: Option<Vec<(String, Vec<Diagnostic>)>>,
    /// ワークスペース層のキーマップ (`finish_workspace_keymap` で当てる) が、自動の読み直しの回のもの。
    pub(crate) ws_keymap_auto: bool,
    /// `.editorconfig` の読み直しの回 (`EditorConfigTrigger::Reload` の `round`) のうち、自動の読み直しの回。
    pub(crate) auto_ec_round: Option<u64>,
    /// 設定のディレクトリを失効の後に張り直した時刻 (続けて失効するディレクトリを、読み直しのたびに張り直さない)。
    renewed: BTreeMap<FsPath, Tick>,
    /// 最後に当てた読み直しの覚え (キーマップを当てるときに使う)。
    pub(crate) run: Option<RunInfo>,
}

impl EditorState {
    /// 設定を監視するか (RLD-4): Port があり、Host が読込元を渡していて、`file.watch.enabled`。
    fn config_watch_wanted(&self) -> bool {
        self.watch.watcher.is_some()
            && self.io.config_sources.is_some()
            && self.config().watch_enabled
    }

    /// 張る条件が変わったかを見て、張る・外す (`watch_reconfigure`・読込元を置いたとき)。
    pub(crate) fn config_watch_reconfigure(&mut self) {
        let want = self.config_watch_wanted();
        if want == self.watch.config.on {
            return;
        }
        if want {
            self.watch.config.on = true;
            self.config_watch_refresh();
        } else {
            self.config_watch_clear();
        }
    }

    /// 監視の対象を解き直す (監視しているときだけ)。同じ対象の解き直しが結果を待っていれば積まない。
    pub(crate) fn config_watch_refresh(&mut self) {
        if !self.watch.config.on {
            return;
        }
        let targets = self.config_watch_targets();
        if self
            .watch
            .config
            .resolving
            .as_ref()
            .is_some_and(|(_, t)| *t == targets)
        {
            return;
        }
        self.watch.config.generation += 1;
        let generation = self.watch.config.generation;
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::ResolveConfigWatch {
                targets: targets.clone(),
            },
        );
        self.io
            .pending
            .insert(job, Pending::ConfigWatchResolve { generation });
        self.watch.config.resolving = Some((generation, targets));
    }

    /// 見るファイルとディレクトリ (RLD-1)。`--no-config` の読込元は空で、`--no-workspace-config` はワークスペース層を含めない。
    fn config_watch_targets(&self) -> Vec<ConfigWatchTarget> {
        let Some(src) = self.io.config_sources.as_ref() else {
            return Vec::new();
        };
        let mut out: Vec<ConfigWatchTarget> = src
            .files
            .iter()
            .map(|(_, p)| ConfigWatchTarget::File(p.clone()))
            .collect();
        if let Some(k) = &src.keymap {
            out.push(ConfigWatchTarget::File(k.clone()));
        }
        // 使っているテーマファイル (組込みのテーマなら見ない)。`inherits` の親も見るよう、置き場所の `*.toml` を見る
        let theme = {
            let c = self.config();
            eve_config::theme::with_background(&c.theme_name, c.theme_background).to_owned()
        };
        if let Some(dir) = &src.themes
            && eve_config::theme::builtin(&theme).is_none()
        {
            out.push(ConfigWatchTarget::File(dir.join(format!("{theme}.toml"))));
            out.push(ConfigWatchTarget::TomlDir(dir.clone()));
        }
        if let Some(dir) = &src.languages {
            out.push(ConfigWatchTarget::TomlDir(dir.clone()));
        }
        if !src.no_workspace_config
            && let Some(root) = self.workspace().map(|w| w.root().clone())
        {
            for f in crate::io::workspace_config::workspace_files(&root) {
                out.push(ConfigWatchTarget::File(f));
            }
        }
        out
    }

    /// 解き直しの結果: 監視の表の設定の印を、解いた対象に合わせる (要らなくなったディレクトリは、文書が無ければ外す。
    /// 失効した・無かったディレクトリは張り直す)。
    pub(crate) fn finish_config_watch_resolve(&mut self, generation: u64, r: JobResult) {
        if self
            .watch
            .config
            .resolving
            .as_ref()
            .is_none_or(|(g, _)| *g != generation)
        {
            // 古い解き直し (後に積んだものが勝つ)・止めた後に届いた
            return;
        }
        self.watch.config.resolving = None;
        let JobResult::ConfigWatchResolved { dirs } = r else {
            return;
        };
        if !self.watch.config.on {
            return;
        }
        let mut want: BTreeMap<FsPath, ConfigNames> = BTreeMap::new();
        for d in dirs {
            let e = want.entry(d.dir).or_insert_with(|| ConfigNames {
                names: BTreeSet::new(),
                any_toml: false,
                shown: d.shown.clone(),
            });
            match d.name {
                Some(n) => {
                    e.names.insert(name_key(&n));
                }
                None => e.any_toml = true,
            }
        }
        let stale: Vec<FsPath> = self
            .watch
            .dirs
            .iter()
            .filter(|(dir, e)| e.config.is_some() && !want.contains_key(*dir))
            .map(|(dir, _)| dir.clone())
            .collect();
        for dir in stale {
            self.config_watch_drop_dir(&dir);
        }
        let now = self.now;
        for (dir, names) in want {
            match self.watch.dirs.get_mut(&dir) {
                Some(e) => {
                    e.config = Some(names);
                    // 失効した・無かったディレクトリの張り直しは、前の張り直しから `CONFIG_RENEW_MIN` の内なら控える (張り
                    // 直してすぐ失効するディレクトリで、失効 → 読み直し → 張り直しを繰り返さない。次の読み直しで張り直す)
                    let lost = matches!(
                        e.state,
                        DirState::Lost | DirState::Failed(WatchError::NotFound)
                    );
                    if lost {
                        let recent = self
                            .watch
                            .config
                            .renewed
                            .get(&dir)
                            .is_some_and(|t| now < *t + CONFIG_RENEW_MIN);
                        if recent {
                            continue;
                        }
                        self.watch.config.renewed.insert(dir.clone(), now);
                    }
                    self.watch_ensure_dir(&dir);
                }
                None => {
                    let Some(w) = self.watch.watcher.clone() else {
                        return;
                    };
                    let id = w.watch_dir(&dir);
                    self.watch.ids.insert(id, dir.clone());
                    self.watch.dirs.insert(
                        dir,
                        DirWatch {
                            id: Some(id),
                            state: DirState::Requested,
                            docs: BTreeSet::new(),
                            names: BTreeMap::new(),
                            config: Some(names),
                        },
                    );
                }
            }
        }
    }

    /// ディレクトリの設定の印を外す (文書が無ければ監視も外す)。
    fn config_watch_drop_dir(&mut self, dir: &FsPath) {
        self.watch.config.renewed.remove(dir);
        let Some(e) = self.watch.dirs.get_mut(dir) else {
            return;
        };
        e.config = None;
        if !e.docs.is_empty() {
            return;
        }
        if let Some(e) = self.watch.dirs.remove(dir)
            && let Some(id) = e.id
        {
            if let Some(w) = &self.watch.watcher {
                w.unwatch(id);
            }
            self.watch.ids.remove(&id);
        }
    }

    /// 設定の監視を止める (`file.watch.enabled = false`): 設定の印を外し、待ち合わせと、期限の来た自動の読み直しを捨てる
    /// (文書の監視は残す。始めた読み直しは終わらせる)。
    fn config_watch_clear(&mut self) {
        let c = &mut self.watch.config;
        c.on = false;
        c.resolving = None;
        c.wait = None;
        c.queued = false;
        let dirs: Vec<FsPath> = self
            .watch
            .dirs
            .iter()
            .filter(|(_, e)| e.config.is_some())
            .map(|(d, _)| d.clone())
            .collect();
        for dir in dirs {
            self.config_watch_drop_dir(&dir);
        }
    }

    /// 監視の知らせのうち、設定ファイルに当たるもの (`Changed` の見る名前・`Rescan`・`Lost`。文書の名前に当たらない
    /// Windows の短い名前も、名前を直す I/O をせずに当たりとする)。
    pub(crate) fn config_watch_hit(entry: &DirWatch, kind: &crate::io::watch::FsEventKind) -> bool {
        use crate::io::watch::FsEventKind;
        let Some(c) = &entry.config else {
            return false;
        };
        match kind {
            FsEventKind::Changed { name } => c.matches(name) || maybe_short_name(name),
            FsEventKind::Rescan | FsEventKind::Lost => true,
            _ => false,
        }
    }

    /// 設定ファイルの知らせが届いた: 待ち合わせを始める・延ばす (WATCH-4)。
    pub(crate) fn config_watch_arm(&mut self) {
        if !self.watch.config.on {
            return;
        }
        let now = self.now;
        let w = self.watch.config.wait.get_or_insert(Wait {
            first: Some(now),
            last: now,
        });
        w.last = now;
        if w.first.is_none() {
            w.first = Some(now);
        }
    }

    /// 自動の読み直しで読んだ内容を捨てた (読む間にファイルが変わった、RLD-3): 待ち合わせからやり直す (続く知らせがあれば
    /// それで延びる)。
    pub(crate) fn config_watch_rearm(&mut self) {
        self.config_watch_arm();
    }

    /// 設定の待ち合わせの期限 (`watch_deadline` に入れる)。読み直しの最中で延ばしたものは期限を持たない (ジョブの結果で
    /// 再開する。過去の期限で Host を空回りさせない)。
    pub(crate) fn config_watch_deadline(&self) -> Option<Tick> {
        self.watch.config.wait.map(Wait::due)
    }

    /// 期限が来たら読み直す (`watch_tick` から)。打ち切りの読み直しは待ち合わせの期限を消さない (WATCH-4)。
    pub(crate) fn config_watch_tick(&mut self) {
        let now = self.now;
        let Some(w) = self.watch.config.wait.as_mut() else {
            return;
        };
        if now >= w.settle_at() {
            self.watch.config.wait = None;
        } else if w.cap_at().is_some_and(|c| now >= c) {
            w.first = None;
        } else {
            return;
        }
        if self.config_reload_busy() {
            self.watch.config.queued = true;
        } else {
            self.start_auto_reload();
        }
    }

    /// ジョブの結果を受けた後: 延ばした読み直しを、前の読み直しの連鎖が終わっていれば始める (手動を先にする。手動の読み直しは
    /// 自動の分も読む)。
    pub(crate) fn config_watch_resume(&mut self) {
        if self.config_reload_busy() {
            return;
        }
        let c = &self.watch.config;
        // 連鎖が終わり、続きの読み直し (待ち合わせ・順番待ち) も無い: 連鎖の前の一覧を捨てる
        if c.chain_diags.is_some() && c.wait.is_none() && !c.queued {
            self.watch.config.chain_diags = None;
        }
        let c = &self.watch.config;
        if !(c.queued || c.queued_manual) {
            return;
        }
        let manual = std::mem::take(&mut self.watch.config.queued_manual);
        let auto = std::mem::take(&mut self.watch.config.queued);
        if manual {
            self.spawn_config_reload(false);
        } else if auto && self.watch.config.on {
            self.start_auto_reload();
        }
    }

    /// 設定の読み直しの連鎖 (設定ファイル・ワークスペース層・テーマ・言語定義の読込) のジョブがある。
    pub(crate) fn config_reload_busy(&self) -> bool {
        self.io.pending.values().any(|p| {
            matches!(
                p,
                Pending::ConfigReload { .. }
                    | Pending::WorkspaceConfig { .. }
                    | Pending::ThemeRead { .. }
                    | Pending::LanguagesRead { .. }
                    | Pending::LanguagesPrecompile { .. }
            )
        })
    }

    /// `config.reload` が自動の読み直しの最中に来た: 終わってから手動で読み直す。
    pub(crate) fn config_watch_queue_manual(&mut self) {
        self.watch.config.queued_manual = true;
    }

    fn start_auto_reload(&mut self) {
        // 連鎖の前の診断の一覧を覚える (前の回の続きなら、その回の前の一覧のまま)
        if self.watch.config.chain_diags.is_none() {
            self.watch.config.chain_diags = Some(self.ui.config_diagnostics.clone());
        }
        self.spawn_config_reload(true);
    }

    /// `msg.config.reload_kept` を出すか (RLD-3): 自動の読み直しでは、同じファイルの同じ内容で 1 回だけ。手動では毎回出し、
    /// 出した内容を覚える。
    pub(crate) fn config_kept_once(&mut self, file: &str, digest: Option<u64>, auto: bool) -> bool {
        let seen = self.watch.config.kept.insert(file.to_owned(), digest);
        !auto || seen != Some(digest)
    }

    /// 誤り無く読めたファイルの `reload_kept` の覚えを捨てる (また同じ内容で壊れたら知らせる)。
    pub(crate) fn config_kept_clear(&mut self, file: &str) {
        self.watch.config.kept.remove(file);
    }

    /// 読み直しの知らせ (`reload_kept`・要約など)。自動の読み直しでは、当てる前に表示していた警告・エラーを置き換えない
    /// (メッセージの履歴には積む)。
    pub(crate) fn show_reload_message(
        &mut self,
        auto: bool,
        level: MessageLevel,
        key: &str,
        args: &[(&str, &str)],
    ) {
        let quiet = auto
            && self
                .watch
                .config
                .scope
                .as_ref()
                .map_or_else(|| self.alert_to_keep(), |s| s.alert);
        if quiet {
            // 履歴にだけ積む (表示中の警告・エラーを置き換えない)
            self.show_background_message(level, key, args);
        } else {
            self.show_message(level, key, args);
        }
    }

    /// 自動の読み直しの結果を当てる間の覚えを置き、前の覚えを返す (`end_reload_scope` で戻す)。
    pub(crate) fn begin_reload_scope(&mut self) -> Option<ReloadScope> {
        let alert = self
            .watch
            .config
            .scope
            .as_ref()
            .map_or_else(|| self.alert_to_keep(), |s| s.alert);
        let scope = ReloadScope { alert };
        self.watch.config.scope.replace(scope)
    }

    /// 表示中のメッセージが、自動の読み直しの知らせで置き換えない警告・エラーか。設定の読込の警告 (前の読み直しの
    /// `reload_kept`・診断の要約) は、新しい読み直しの結果で置き換えてよい (直した後に古い誤りの知らせを残さない)。
    fn alert_to_keep(&self) -> bool {
        const RELOAD_KEYS: [&str; 4] = [
            keys::MSG_CONFIG_RELOAD_KEPT,
            keys::MSG_CONFIG_RELOAD_BUSY_KEPT,
            keys::MSG_CONFIG_PROBLEM,
            keys::MSG_CONFIG_PROBLEMS,
        ];
        self.message_is_alert()
            && !self
                .shown_message_key()
                .is_some_and(|k| RELOAD_KEYS.contains(&k))
    }

    /// `begin_reload_scope` の前の覚えに戻す。
    pub(crate) fn end_reload_scope(&mut self, outer: Option<ReloadScope>) {
        self.watch.config.scope = outer;
    }

    /// 自動の読み直しの結果を当てる間だけ、知らせ方を変える (`f` の間。当てる前の一覧と同じ診断の要約は出さず、当てる前に
    /// 表示していた警告・エラーを置き換えない)。`auto` でなければ `f` をそのまま呼ぶ。
    pub(crate) fn with_reload_scope<R>(&mut self, auto: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        if !auto {
            return f(self);
        }
        let outer = self.begin_reload_scope();
        let r = f(self);
        self.end_reload_scope(outer);
        r
    }

    /// 診断の要約の出し方 (`report_config_with`・`report_config_more` から)。自動の読み直しの結果を当てている間は、連鎖を
    /// 始める前の一覧 (段ごとの一覧ではない。前の段が消して後の段が出し直す診断があるため) に同じ診断があれば出さず (`None`)、当てる前に (読み直しのものでない) 警告・エラーを表示していれば履歴にだけ
    /// 積み、ほかは表示する。
    pub(crate) fn reload_delivery(
        &self,
        file: &str,
        diagnostics: &[Diagnostic],
        delivery: Delivery,
    ) -> Option<Delivery> {
        let Some(scope) = &self.watch.config.scope else {
            return Some(delivery);
        };
        let before = self.watch.config.chain_diags.as_deref().unwrap_or_default();
        let same = before
            .iter()
            .find(|(f, _)| f == file)
            .is_some_and(|(_, ds)| diagnostics.iter().all(|d| ds.contains(d)));
        if same {
            None
        } else if scope.alert {
            Some(Delivery::Background)
        } else {
            // 置き換えてよい知らせだけが出ている (前の読み直しの警告を含む)
            Some(Delivery::Replace)
        }
    }
}
