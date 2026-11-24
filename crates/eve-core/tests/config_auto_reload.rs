//! 設定ファイルの自動の再読込 (FR-706、docs/07 §6 の RLD-1〜RLD-5、03 §9.5.1 の WATCH-12。v0.5 の M33 の段 9)。
//!
//! 偽の監視 (`FakeWatcher`) で知らせを入れ、手動の時計で待ち合わせ (100 ms / 最長 1 s、WATCH-4) を越える。設定の
//! ディレクトリは `/evo/config`、ワークスペースのルートは `/w/repo` (`--workspace`)。

use eve_config::{Conditions, ExternalChangePolicy, Frontend, Os};
use eve_core::{
    ConfigSources, FsEventKind, FsPath, InitialFile, StartupConfig, WATCH_SETTLE, WATCH_SETTLE_MAX,
    WatchError, WatchId,
};
use eve_testkit::{FakeWatcher, Harness, WatchRequest};
use eve_view::MessageLevel;

const DIR: &str = "/evo/config";
const CONFIG: &str = "/evo/config/config.toml";
const KEYMAP: &str = "/evo/config/keymap.toml";
const THEMES: &str = "/evo/config/themes";
const LANGS: &str = "/evo/config/languages";
const WS_DIR: &str = "/w/repo/.eve";
const WS_CONFIG: &str = "/w/repo/.eve/config.toml";
const IN_ROOT: &str = "/w/repo/a.txt";
/// 起動時に開いた文書のディレクトリ (文書の監視)。
const ROOT_DIR: &str = "/w/repo";

/// 起動の材料。
struct Setup<'a> {
    user: &'a str,
    /// ほかに置くファイル (パス, 中身)。
    files: &'a [(&'a str, &'a [u8])],
    /// 置くディレクトリ。
    dirs: &'a [&'a str],
    /// `--config` で渡すファイル (空なら `<DIR>/config.toml`)。
    config_files: &'a [&'a str],
    no_config: bool,
    no_workspace_config: bool,
    watcher: FakeWatcher,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Setup {
            user: "",
            files: &[],
            dirs: &[],
            config_files: &[],
            no_config: false,
            no_workspace_config: false,
            watcher: FakeWatcher::new().auto_started(),
        }
    }
}

fn linux() -> Conditions {
    Conditions {
        os: Some(Os::Linux),
        frontend: Frontend::Tui,
        ..Conditions::default()
    }
}

fn start(s: &Setup<'_>) -> (Harness, FakeWatcher) {
    let fake = s.watcher.clone();
    let mut b = Harness::builder()
        .defer_start()
        .watcher(fake.clone())
        .dir(DIR)
        .dir("/w/repo/.git")
        .file(CONFIG, s.user.as_bytes())
        .file(IN_ROOT, b"text\n");
    for d in s.dirs {
        b = b.dir(d);
    }
    for (p, data) in s.files {
        b = b.file(p, data);
    }
    let mut h = b.build();
    let files: Vec<(String, FsPath)> = if s.no_config {
        Vec::new()
    } else if s.config_files.is_empty() {
        vec![("config.toml".to_owned(), FsPath::new(CONFIG))]
    } else {
        s.config_files
            .iter()
            .map(|f| ((*f).to_owned(), FsPath::new(f)))
            .collect()
    };
    let sources: Vec<eve_config::Source> = files
        .iter()
        .map(|(name, p)| eve_config::Source {
            name: name.clone(),
            text: Ok(h
                .fs
                .read(&p.display())
                .map(|b| String::from_utf8(b).expect("UTF-8"))),
        })
        .collect();
    let resolved = eve_config::resolve(&sources, &[], linux());
    let user_dir = |sub: &str| (!s.no_config).then(|| FsPath::new(DIR).join(sub));
    h.editor.configure(StartupConfig {
        config: Some(resolved.config),
        config_origins: Some(resolved.origins),
        config_user: Some(resolved.user),
        config_sources: Some(ConfigSources {
            files,
            keymap: (!s.no_config).then(|| FsPath::new(KEYMAP)),
            sets: Vec::new(),
            themes: user_dir("themes"),
            languages: user_dir("languages"),
            locales: None,
            conditions: linux(),
            no_workspace_config: s.no_config || s.no_workspace_config,
            home: None,
        }),
        workspace: Some("/w/repo".to_owned()),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path(IN_ROOT)], false, None);
    h.started();
    h.run_jobs();
    h.pump_fs();
    h.run_jobs();
    h.clear_log();
    (h, fake)
}

fn id_of(fake: &FakeWatcher, dir: &str) -> WatchId {
    fake.id_of(dir)
        .unwrap_or_else(|| panic!("{dir} を監視している: {:?}", fake.active()))
}

/// 外で書いた (中身を替えて、名前の知らせを入れる)。
fn write(h: &mut Harness, fake: &FakeWatcher, path: &str, data: &[u8]) {
    h.fs.add_file(path, data);
    let (dir, name) = path.rsplit_once('/').expect("パス");
    fake.changed(id_of(fake, dir), name);
    h.pump_fs();
}

/// 待ち合わせを越えて読み直しを走らせる。
fn settle(h: &mut Harness) {
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
}

fn tab(h: &Harness) -> u8 {
    h.editor.state().config().tab_width
}

fn count(h: &Harness, key: &str) -> usize {
    h.messages().iter().filter(|m| *m == key).count()
}

fn watched_dirs(fake: &FakeWatcher) -> Vec<String> {
    // Windows の `join` は `\` で区切る (比べるために `/` に揃える)
    let mut v: Vec<String> = fake
        .active()
        .iter()
        .map(|(_, d)| d.display().replace('\\', "/"))
        .collect();
    v.sort();
    v
}

// ---------- AC-1: 書き換えて保存すると読み直し、変わった項目を要約する ----------

// verifies: AC-FR-706-1
#[test]
fn 外で書き換えた設定ファイルを待ち合わせの後に読み直し_要約を出す() {
    let (mut h, fake) = start(&Setup::default());
    assert!(
        watched_dirs(&fake).contains(&DIR.to_owned()),
        "{:?}",
        watched_dirs(&fake)
    );
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 8\n");
    // 待ち合わせの前は読まない (WATCH-4)
    h.advance_ms(WATCH_SETTLE.0 - 1);
    h.run_jobs();
    assert_eq!(tab(&h), 4);
    h.advance_ms(1);
    h.run_jobs();
    assert_eq!(tab(&h), 8);
    assert_eq!(
        h.messages(),
        vec!["info.config.reloaded"],
        "手動のときと同じ要約"
    );
    // 期限は残らない (Host を空回りさせない)
    assert_eq!(h.editor.next_deadline(), None);
}

// verifies: AC-FR-706-1
#[test]
fn 多数の知らせは待ち合わせで_1_回の読み直しにまとめる() {
    let (mut h, fake) = start(&Setup::default());
    let id = id_of(&fake, DIR);
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    for i in 0..50 {
        fake.changed(id, if i % 2 == 0 { "config.toml" } else { "x.tmp" });
        fake.push(id, FsEventKind::Rescan);
        h.pump_fs();
        h.advance_ms(10);
    }
    settle(&mut h);
    assert_eq!(tab(&h), 2);
    assert_eq!(count(&h, "info.config.reloaded"), 1, "{:?}", h.messages());
}

// verifies: AC-FR-706-1
#[test]
fn 書き続けても最初の知らせから_1_秒で読み直す() {
    let (mut h, fake) = start(&Setup::default());
    let id = id_of(&fake, DIR);
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    let mut waited = 0;
    while waited < WATCH_SETTLE_MAX.0 {
        fake.changed(id, "config.toml");
        h.pump_fs();
        h.advance_ms(50);
        h.run_jobs();
        waited += 50;
    }
    assert_eq!(tab(&h), 2, "打ち切りで読み直した");
    // 打ち切りの後の待ち合わせの期限でも、もう 1 回読む (変わっていなければ何も知らせない)
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 3\n");
    fake.changed(id, "config.toml");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 3);
}

// verifies: AC-FR-706-1
#[test]
fn 内容が前と同じなら何も知らせず_ほかの名前の知らせでは読まない() {
    let (mut h, fake) = start(&Setup {
        user: "[editor]\ntab_width = 8\n",
        ..Setup::default()
    });
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 8\n");
    settle(&mut h);
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    // 見ていない名前 (一時ファイル・ほかのファイル) の知らせでは読み直さない
    let jobs_before = h.pending_jobs();
    fake.changed(id_of(&fake, DIR), "config.toml.tmp~");
    h.pump_fs();
    assert_eq!(h.editor.next_deadline(), None, "待ち合わせを始めない");
    assert_eq!(h.pending_jobs(), jobs_before);
}

// verifies: AC-FR-706-1
#[test]
fn 自分の保存でも読み直し_文書の監視と同じディレクトリの要求を共有する() {
    let (mut h, fake) = start(&Setup::default());
    h.open(CONFIG);
    h.run_jobs();
    h.pump_fs();
    assert_eq!(
        fake.active()
            .iter()
            .filter(|(_, d)| d.display() == DIR)
            .count(),
        1,
        "同じディレクトリは 1 つの要求: {:?}",
        fake.active()
    );
    h.text("[editor]\ntab_width = 6\n");
    h.cmd("file.save");
    h.run_jobs();
    h.clear_log();
    // 保存の知らせ (自分の保存を抑えない、RLD-2)
    fake.changed(id_of(&fake, DIR), "config.toml");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 6);
    assert!(
        h.messages().iter().any(|m| m == "info.config.reloaded"),
        "{:?}",
        h.messages()
    );
    assert!(
        !h.messages().iter().any(|m| m.starts_with("info.external.")),
        "文書の監視は自分の保存を知らせない: {:?}",
        h.messages()
    );
}

// verifies: AC-FR-706-1
#[test]
fn 構文エラーでは前の設定を保ち_同じ内容の誤りは_1_回だけ知らせる() {
    let (mut h, fake) = start(&Setup {
        user: "[editor]\ntab_width = 8\n",
        ..Setup::default()
    });
    write(&mut h, &fake, CONFIG, b"[editor\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(tab(&h), 8, "前の設定のまま");
    assert_eq!(count(&h, "msg.config.reload_kept"), 1, "{:?}", h.messages());
    assert_eq!(count(&h, "msg.config.problem"), 1, "{:?}", h.messages());
    // 同じ内容の知らせ (同期ソフトの書き直し)・同じディレクトリのほかのファイルの変更では出し直さない
    h.clear_log();
    write(&mut h, &fake, CONFIG, b"[editor\ntab_width = 2\n");
    settle(&mut h);
    write(&mut h, &fake, KEYMAP, b"");
    settle(&mut h);
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    // 直したら読み直し (前の読み直しの警告は、新しい結果の要約で置き換える)、また同じ内容で壊したら知らせる
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(tab(&h), 2);
    assert_eq!(h.messages(), vec!["info.config.reloaded"]);
    assert_eq!(
        h.frame().messages.current.map(|m| m.level),
        Some(MessageLevel::Info)
    );
    h.clear_log();
    write(&mut h, &fake, CONFIG, b"[editor\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(count(&h, "msg.config.reload_kept"), 1, "{:?}", h.messages());
    // 手動の `config.reload` は毎回知らせる
    h.clear_log();
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(count(&h, "msg.config.reload_kept"), 1, "{:?}", h.messages());
}

// verifies: AC-FR-706-1
#[test]
fn キーマップ_テーマファイル_言語定義の変更でも読み直す() {
    let (mut h, fake) = start(&Setup {
        user: "[theme]\nname = \"mine\"\n",
        files: &[
            (
                "/evo/config/themes/mine.toml",
                b"[meta]\ninherits = \"base\"\n",
            ),
            (
                "/evo/config/themes/base.toml",
                b"[ui]\nselection = { bg = \"#112233\" }\n",
            ),
            ("/evo/config/languages/x.toml", b""),
        ],
        ..Setup::default()
    });
    let dirs = watched_dirs(&fake);
    for d in [DIR, THEMES, LANGS, WS_DIR] {
        if d == WS_DIR {
            assert!(!dirs.contains(&d.to_owned()), "無いディレクトリは見ない");
        } else {
            assert!(dirs.contains(&d.to_owned()), "{d}: {dirs:?}");
        }
    }
    write(
        &mut h,
        &fake,
        KEYMAP,
        b"[keys.editor]\n\"f5\" = \"file.save\"\n",
    );
    settle(&mut h);
    assert!(
        h.messages().iter().any(|m| m == "info.config.reloaded"),
        "keymap.toml: {:?}",
        h.messages()
    );
    // テーマファイル (`inherits` の親も見る) と言語定義: 設定のキーは変わらないので要約は出さず、読み直して当てる
    for (path, data) in [
        (
            "/evo/config/themes/base.toml",
            &b"[ui]\nselection = { bg = \"#445566\" }\n"[..],
        ),
        (
            "/evo/config/languages/y.toml",
            b"[[language]]\nname = \"y\"\nfile_types = [\"yy\"]\n",
        ),
    ] {
        h.clear_log();
        let jobs = h.pending_jobs();
        write(&mut h, &fake, path, data);
        h.advance_ms(WATCH_SETTLE.0);
        assert!(h.pending_jobs() > jobs, "{path} で読み直しを積む");
        h.run_jobs();
        assert!(h.messages().is_empty(), "{path}: {:?}", h.messages());
    }
    assert!(
        h.editor.state().languages().get("y").is_some(),
        "読み直した言語定義を当てた"
    );
    assert_eq!(
        h.frame().roles.get("ui.selection").bg,
        eve_view::Color::Rgb(0x44, 0x55, 0x66),
        "継いだ親のテーマファイルの変更も当てた"
    );
}

// verifies: AC-FR-706-1
#[test]
fn ワークスペース層の変更でも読み直し_準安全と危険な項目は保留のまま() {
    let (mut h, fake) = start(&Setup {
        files: &[(WS_CONFIG, b"[editor]\ntab_width = 2\n")],
        ..Setup::default()
    });
    assert_eq!(tab(&h), 2, "起動時のワークスペース層");
    write(
        &mut h,
        &fake,
        WS_CONFIG,
        b"[editor]\ntab_width = 3\n[theme]\nname = \"x\"\n[file.swap]\nexclude_secret_files = false\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 3);
    assert!(
        h.messages().iter().any(|m| m == "info.config.reloaded"),
        "{:?}",
        h.messages()
    );
    let c = h.editor.state().config();
    assert_eq!(
        c.theme_name, "default",
        "準安全の項目は読み直しでも保留 (RLD-5)"
    );
    assert!(
        c.swap_exclude_secret_files,
        "危険な項目は読み直しでも当てない (RLD-5)"
    );
}

// verifies: AC-FR-706-1
#[test]
fn config_で渡したファイルの親を見る() {
    let (mut h, fake) = start(&Setup {
        files: &[("/w/dot/eve.toml", b"[editor]\ntab_width = 4\n")],
        config_files: &["/w/dot/eve.toml"],
        ..Setup::default()
    });
    write(
        &mut h,
        &fake,
        "/w/dot/eve.toml",
        b"[editor]\ntab_width = 5\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 5);
    // `<DIR>/config.toml` は読まないので、その知らせでは読み直さない (`keymap.toml` のために DIR は見る)
    h.clear_log();
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 7\n");
    assert_eq!(h.editor.next_deadline(), None);
}

// ---------- RLD-3: 読む間に変わったら捨てる・Undo を減らす値は効かせない ----------

// verifies: AC-FR-706-1
#[test]
fn 読む間にファイルが変わったら読んだ内容を捨てて読み直す() {
    let (mut h, fake) = start(&Setup::default());
    // 書きかけ (行の境目までで構文として正しい) を読んでいる間に、残りが書かれた
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    h.fs.write_during_read(CONFIG, b"[editor]\ntab_width = 2\nindent_width = 2\n");
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert_eq!(tab(&h), 4, "途中の内容は当てない");
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    assert!(h.editor.next_deadline().is_some(), "読み直しを待つ");
    settle(&mut h);
    let c = h.editor.state().config();
    assert_eq!((c.tab_width, c.indent_width), (2, 2));
}

// verifies: AC-FR-706-1
#[test]
fn 読む間にワークスペース層が変わったら読んだ内容を捨てる() {
    let (mut h, fake) = start(&Setup {
        files: &[(WS_CONFIG, b"")],
        ..Setup::default()
    });
    write(&mut h, &fake, WS_CONFIG, b"[editor]\ntab_width = 2\n");
    h.fs.write_during_read(WS_CONFIG, b"[editor]\ntab_width = 3\n");
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert_eq!(tab(&h), 4, "途中の内容は当てない");
    settle(&mut h);
    assert_eq!(tab(&h), 3);
}

// verifies: AC-FR-706-1
#[test]
fn 自動の読み直しでは_undo_の上限を下げる変化を効かせない() {
    let (mut h, fake) = start(&Setup::default());
    let before = h.editor.state().config().undo_total_max_bytes;
    let show_origin_before = show_origin(&mut h, "total_max_bytes");
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[editor.undo]\ntotal_max_bytes = 1048576\n[editor]\ntab_width = 2\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 2, "ほかの項目は当てる");
    assert_eq!(h.editor.state().config().undo_total_max_bytes, before);
    let summary = message_text(&mut h);
    assert!(
        summary.contains("editor.tab_width") && !summary.contains("editor.undo"),
        "効かせなかった項目は要約に出さない: {summary}"
    );
    // `config.show` の由来も前のまま (効いていない値の由来を出さない)
    assert_eq!(
        show_origin(&mut h, "total_max_bytes"),
        show_origin_before,
        "由来は前のまま"
    );
    // ほかの項目だけを変えた読み直しでも、ユーザー層の小さい値は効かない
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[editor.undo]\ntotal_max_bytes = 1048576\n[editor]\ntab_width = 3\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 3);
    assert_eq!(h.editor.state().config().undo_total_max_bytes, before);
    // 手動の `config.reload` では効く
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(h.editor.state().config().undo_total_max_bytes, 1_048_576);
    // 上げる変化は自動でも効く
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[editor.undo]\ntotal_max_bytes = 2097152\n",
    );
    settle(&mut h);
    assert_eq!(h.editor.state().config().undo_total_max_bytes, 2_097_152);
    assert_ne!(
        show_origin(&mut h, "total_max_bytes"),
        show_origin_before,
        "効いた値の由来は設定ファイル"
    );
}

/// 今のメッセージの本文。
fn message_text(h: &mut Harness) -> String {
    h.frame()
        .messages
        .current
        .map(|m| m.text.spans.iter().map(|s| s.text.as_str()).collect())
        .unwrap_or_default()
}

/// `config.show` の一覧で `query` に当たる最初の行の右の列 (値と由来)。
fn show_origin(h: &mut Harness, query: &str) -> String {
    h.cmd("config.show");
    h.text(query);
    let f = h.frame();
    let Some(eve_view::OverlayModel::List { items, .. }) = f.overlays.first() else {
        panic!("一覧が出ていない");
    };
    let row = items
        .first()
        .map(|i| {
            let label: String = i.label.spans.iter().map(|s| s.text.as_str()).collect();
            let detail: String = i
                .detail
                .as_ref()
                .map(|d| d.spans.iter().map(|s| s.text.as_str()).collect())
                .unwrap_or_default();
            format!("{label} | {detail}")
        })
        .unwrap_or_default();
    h.cmd("ui.cancel");
    row
}

// ---------- 読み直しの最中の契機・手動との重なり ----------

// verifies: AC-FR-706-1
#[test]
fn 読み直しの最中に来た契機は_終わった後にもう_1_回読む() {
    let (mut h, fake) = start(&Setup {
        files: &[(WS_CONFIG, b"")],
        ..Setup::default()
    });
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    h.advance_ms(WATCH_SETTLE.0);
    // ユーザー層を読んだ (ワークスペース層の読込を待っている) 間に、次の書込の知らせが来て期限を過ぎた
    assert!(h.run_one_job());
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 3\n");
    fake.changed(id_of(&fake, DIR), "config.toml");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(
        h.editor.next_deadline(),
        None,
        "最中は期限を持たない (ジョブの結果で再開する)"
    );
    h.run_jobs();
    assert_eq!(tab(&h), 3, "終わった後に読み直した");
}

// verifies: AC-FR-706-1
#[test]
fn 自動の読み直しの最中の_config_reload_は終わった後に手動で読み直す() {
    let (mut h, fake) = start(&Setup {
        files: &[(WS_CONFIG, b"")],
        ..Setup::default()
    });
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "自動の読み直しを積んだ");
    // 設定ファイルの段の最中と、ワークスペース層の段の最中の両方で `config.reload` を出す
    h.cmd("config.reload");
    assert_eq!(h.pending_jobs(), 1, "すぐには積まない (待たせる)");
    assert!(h.run_one_job());
    h.cmd("config.reload");
    assert_eq!(h.pending_jobs(), 1, "ワークスペース層の段でも待たせる");
    h.run_jobs();
    assert_eq!(tab(&h), 2);
    assert_eq!(
        h.messages(),
        vec!["info.config.reloaded", "info.config.reload_unchanged"],
        "自動の要約の後に、手動が 1 回だけ読み直して知らせる"
    );
}

// verifies: AC-FR-706-1
#[test]
fn 手動の読み直しの最中の_config_reload_も終わった後にもう_1_回読み直す() {
    let (mut h, _fake) = start(&Setup {
        files: &[(WS_CONFIG, b"")],
        ..Setup::default()
    });
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    h.cmd("config.reload");
    assert!(h.run_one_job(), "ユーザー層を読んだ");
    // ワークスペース層を読んでいる段で、書き換えてからもう一度
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 3\n");
    h.cmd("config.reload");
    assert_eq!(h.pending_jobs(), 1, "重ねて始めない");
    h.run_jobs();
    assert_eq!(tab(&h), 3, "待たせた読み直しが後の書換えを読む");
    assert_eq!(count(&h, "info.config.reloaded"), 2, "{:?}", h.messages());
}

// ---------- メッセージの優先 ----------

// verifies: AC-FR-706-1
#[test]
fn 自動の読み直しの要約は表示中の警告を置き換えない() {
    let (mut h, fake) = start(&Setup::default());
    h.editor
        .state_mut()
        .show_message(MessageLevel::Warn, "msg.edit.read_only", &[]);
    // 出した知らせの記録を取り込んでから区切る
    h.pump_fs();
    h.clear_log();
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(tab(&h), 2);
    assert!(
        h.messages().is_empty(),
        "要約は履歴にだけ積む: {:?}",
        h.messages()
    );
    let current = h.frame().messages.current.expect("メッセージ");
    assert_eq!(current.level, MessageLevel::Warn, "警告を残す");
}

// ---------- AC-2: 止める・縮退 ----------

// verifies: AC-FR-706-2
#[test]
fn watch_enabled_を切ると設定も見ず_手動の再読込だけが効く() {
    let (mut h, fake) = start(&Setup::default());
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[file.watch]\nenabled = false\n[editor]\ntab_width = 2\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 2);
    assert!(fake.id_of(DIR).is_none(), "外した: {:?}", fake.active());
    // 外した要求の古い知らせは捨てる
    h.fs.add_file(
        CONFIG,
        b"[file.watch]\nenabled = false\n[editor]\ntab_width = 3\n",
    );
    let old = fake
        .requests()
        .iter()
        .find_map(|r| match r {
            WatchRequest::Watch { id, dir } if dir.display() == DIR => Some(*id),
            _ => None,
        })
        .expect("頼んだ");
    fake.changed(old, "config.toml");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 2, "読み直さない");
    // 手動の `config.reload` は効き、入れ直したら見る
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 5\n");
    h.cmd("config.reload");
    h.run_jobs();
    h.pump_fs();
    assert_eq!(tab(&h), 5);
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 6\n");
    settle(&mut h);
    assert_eq!(tab(&h), 6);
}

// verifies: AC-FR-706-2
#[test]
fn reload_on_external_change_never_でも設定は見る_文書の監視と独立() {
    let (mut h, fake) = start(&Setup::default());
    h.open(CONFIG);
    h.run_jobs();
    h.pump_fs();
    // 文書の監視を止めても、同じディレクトリの設定の監視は残る
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[file]\nreload_on_external_change = \"never\"\n",
    );
    settle(&mut h);
    assert_eq!(
        h.editor.state().config().reload_on_external_change,
        ExternalChangePolicy::Never
    );
    assert!(fake.id_of(DIR).is_some(), "{:?}", fake.active());
    assert!(
        fake.id_of("/w/repo").is_none(),
        "文書だけのディレクトリは外す"
    );
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[file]\nreload_on_external_change = \"never\"\n[editor]\ntab_width = 2\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 2);
    // 同じディレクトリの最後の文書を閉じても、設定の監視は外れない
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    h.cmd("buffer.close");
    h.run_jobs();
    assert_ne!(h.doc_path().as_deref(), Some(CONFIG), "閉じた");
    assert!(fake.id_of(DIR).is_some(), "{:?}", fake.active());
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 3\n");
    settle(&mut h);
    assert_eq!(tab(&h), 3);
}

// verifies: AC-FR-706-2
#[test]
fn 監視できない_fs_の知らせは文書と共有して理由ごとに_1_回_無いディレクトリは知らせない() {
    let fake = FakeWatcher::new();
    let (mut h, fake) = start(&Setup {
        watcher: fake,
        ..Setup::default()
    });
    let id = |d: &str| id_of(&fake, d);
    fake.push(
        id(DIR),
        FsEventKind::Failed {
            err: WatchError::Unsupported {
                fs: "9p".to_owned(),
            },
        },
    );
    fake.push(
        id(ROOT_DIR),
        FsEventKind::Failed {
            err: WatchError::Unsupported {
                fs: "9p".to_owned(),
            },
        },
    );
    h.pump_fs();
    assert_eq!(count(&h, "info.watch.unavailable"), 1, "{:?}", h.messages());
    // 読み直しはしない (知らせが来ても捨てる)
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    fake.changed(id(DIR), "config.toml");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 4);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(tab(&h), 2, "手動の再読込は効く");
}

// verifies: AC-FR-706-2
#[test]
fn 解いた後に消えた設定のディレクトリは知らせずに見ない() {
    let (mut h, fake) = start(&Setup {
        watcher: FakeWatcher::new(),
        dirs: &[LANGS],
        ..Setup::default()
    });
    fake.push(
        id_of(&fake, LANGS),
        FsEventKind::Failed {
            err: WatchError::NotFound,
        },
    );
    h.pump_fs();
    assert!(h.messages().is_empty(), "{:?}", h.messages());
}

// verifies: AC-FR-706-2
#[test]
fn no_config_では設定を見ず_no_workspace_config_ではワークスペース層を見ない() {
    let (_h, fake) = start(&Setup {
        no_config: true,
        files: &[(WS_CONFIG, b"")],
        dirs: &[LANGS, THEMES],
        ..Setup::default()
    });
    assert_eq!(watched_dirs(&fake), vec!["/w/repo".to_owned()], "文書だけ");
    let (_h, fake) = start(&Setup {
        no_workspace_config: true,
        files: &[(WS_CONFIG, b"")],
        ..Setup::default()
    });
    assert!(!watched_dirs(&fake).contains(&WS_DIR.to_owned()));
    assert!(watched_dirs(&fake).contains(&DIR.to_owned()));
}

// verifies: AC-FR-706-1
#[test]
fn 起動時に無かったディレクトリは見ず_テーマの名前が変わったら置き場所を見る() {
    let (mut h, fake) = start(&Setup {
        files: &[(
            "/evo/config/themes/mine.toml",
            b"[ui]\nselection = { bg = \"#112233\" }\n",
        )],
        ..Setup::default()
    });
    let dirs = watched_dirs(&fake);
    assert!(!dirs.contains(&LANGS.to_owned()), "無いディレクトリ");
    assert!(!dirs.contains(&THEMES.to_owned()), "組込みのテーマ");
    write(&mut h, &fake, CONFIG, b"[theme]\nname = \"mine\"\n");
    settle(&mut h);
    h.pump_fs();
    assert!(
        watched_dirs(&fake).contains(&THEMES.to_owned()),
        "{:?}",
        watched_dirs(&fake)
    );
    write(&mut h, &fake, CONFIG, b"");
    settle(&mut h);
    h.pump_fs();
    assert!(!watched_dirs(&fake).contains(&THEMES.to_owned()));
}

// verifies: AC-FR-706-1
#[test]
fn 失効したディレクトリは読み直した後に張り直す() {
    let (mut h, fake) = start(&Setup::default());
    let old = id_of(&fake, DIR);
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    fake.push(old, FsEventKind::Lost);
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 2, "失効も契機");
    h.pump_fs();
    let new = id_of(&fake, DIR);
    assert_ne!(new, old, "張り直した");
    assert!(
        fake.requests().contains(&WatchRequest::Unwatch(old)),
        "{:?}",
        fake.requests()
    );
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 3\n");
    settle(&mut h);
    assert_eq!(tab(&h), 3);
}

// verifies: AC-FR-706-1
#[test]
fn 削除も契機で_消えたときは_config_reload_と同じく既定に戻す() {
    let (mut h, fake) = start(&Setup {
        user: "[editor]\ntab_width = 8\n",
        ..Setup::default()
    });
    h.fs.remove(&FsPath::new(CONFIG));
    fake.changed(id_of(&fake, DIR), "config.toml");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 4);
    assert!(h.messages().iter().any(|m| m == "info.config.reloaded"));
}

// verifies: AC-FR-706-1
#[test]
fn リンクで置いた設定ファイルはリンクの先のディレクトリも見る() {
    let (mut h, fake) = start(&Setup {
        files: &[("/w/dot/config.toml", b"[editor]\ntab_width = 4\n")],
        ..Setup::default()
    });
    h.fs.remove(&FsPath::new(CONFIG));
    h.fs.add_link(CONFIG, "/w/dot/config.toml");
    // 読込元を置き直した (起動時と同じ): リンクの先を解く
    h.cmd("config.reload");
    h.run_jobs();
    h.pump_fs();
    assert!(
        watched_dirs(&fake).contains(&"/w/dot".to_owned()),
        "{:?}",
        watched_dirs(&fake)
    );
    // リンクの先を書き換えた (dotfiles の管理で、リンクの先のファイルを直接直した)
    write(
        &mut h,
        &fake,
        "/w/dot/config.toml",
        b"[editor]\ntab_width = 7\n",
    );
    settle(&mut h);
    assert_eq!(tab(&h), 7);
}

// verifies: AC-FR-706-1
#[test]
fn 登録の結果より先に届いた知らせでも読み直す() {
    let (mut h, fake) = start(&Setup {
        watcher: FakeWatcher::new(),
        ..Setup::default()
    });
    // `Started` はまだ届いていない (02 §4.4 の知らせの順の約束)
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(tab(&h), 2);
}

// verifies: AC-FR-706-2
#[test]
fn 設定で見なくなったディレクトリも_文書があれば監視を残す() {
    let (mut h, fake) = start(&Setup {
        user: "[theme]\nname = \"mine\"\n",
        files: &[(
            "/evo/config/themes/mine.toml",
            b"[ui]\nselection = { bg = \"#112233\" }\n",
        )],
        ..Setup::default()
    });
    h.open("/evo/config/themes/mine.toml");
    h.run_jobs();
    h.pump_fs();
    let id = id_of(&fake, THEMES);
    // 組込みのテーマに替えた: 設定では `themes/` を見ないが、開いている文書の監視は残る
    write(&mut h, &fake, CONFIG, b"");
    settle(&mut h);
    h.pump_fs();
    assert_eq!(fake.id_of(THEMES), Some(id), "{:?}", fake.active());
    // `themes/` の知らせでは設定を読み直さない (文書の監視だけ)
    let jobs = h.pending_jobs();
    fake.changed(id, "other.toml");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), jobs);
}

// verifies: AC-FR-706-1
#[cfg(windows)]
#[test]
fn windows_では名前の大文字小文字を区別せずに見る() {
    let (mut h, fake) = start(&Setup::default());
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    fake.changed(id_of(&fake, DIR), "CONFIG.TOML");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(tab(&h), 2);
}

// verifies: AC-FR-706-1
#[test]
fn 読めない設定ファイルは前の設定を保つ() {
    let (mut h, fake) = start(&Setup {
        user: "[editor]\ntab_width = 8\n",
        ..Setup::default()
    });
    h.fs.fail_next(
        eve_testkit::FsOp::OpenRead,
        eve_core::FsErrorKind::PermissionDenied,
    );
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(tab(&h), 8, "前の設定のまま");
    assert_eq!(count(&h, "msg.config.reload_kept"), 1, "{:?}", h.messages());
    // 読めるようになった次の知らせで読み直す
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(tab(&h), 2);
}

// ---------- 段 9 のレビュー: 読む間に変わったら捨てる (キーマップ・テーマ・言語定義) ----------

// verifies: AC-FR-706-1
#[test]
fn 読む間にキーマップ_テーマ_言語定義が変わったら当てずに読み直す() {
    const MINE: &str = "/evo/config/themes/mine.toml";
    const X: &str = "/evo/config/languages/x.toml";
    let (mut h, fake) = start(&Setup {
        user: "[theme]\nname = \"mine\"\n",
        files: &[
            (MINE, b"[ui]\nselection = { bg = \"#112233\" }\n"),
            (X, b"[[language]]\nname = \"x\"\nfile_types = [\"xx\"]\n"),
        ],
        ..Setup::default()
    });
    let selection = |h: &mut Harness| h.frame().roles.get("ui.selection").bg;
    // `keymap.toml`: 書きかけを読んでいる間に残りが書かれた
    write(
        &mut h,
        &fake,
        KEYMAP,
        b"[keys.editor]\n\"f5\" = \"file.save\"\n",
    );
    h.fs.write_during_read(KEYMAP, b"[keys.editor]\n\"f5\" = \"file.save_as\"\n");
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert!(
        h.messages().is_empty(),
        "途中の内容は当てない: {:?}",
        h.messages()
    );
    settle(&mut h);
    assert_eq!(count(&h, "info.config.reloaded"), 1, "{:?}", h.messages());
    // テーマファイル
    h.clear_log();
    write(
        &mut h,
        &fake,
        MINE,
        b"[ui]\nselection = { bg = \"#223344\" }\n",
    );
    h.fs.write_during_read(MINE, b"[ui]\nselection = { bg = \"#445566\" }\n");
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert_eq!(
        selection(&mut h),
        eve_view::Color::Rgb(0x11, 0x22, 0x33),
        "途中の内容は当てない"
    );
    assert!(h.editor.next_deadline().is_some(), "読み直しを待つ");
    settle(&mut h);
    assert_eq!(selection(&mut h), eve_view::Color::Rgb(0x44, 0x55, 0x66));
    // 言語定義: 読んでいる間に同じ置き場所にファイルが増えた (並びが変わった)
    write(
        &mut h,
        &fake,
        X,
        b"[[language]]\nname = \"x\"\nfile_types = [\"xy\"]\n",
    );
    h.fs.write_other_during_read(
        X,
        "/evo/config/languages/z.toml",
        b"[[language]]\nname = \"z\"\nfile_types = [\"zz\"]\n",
    );
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert!(
        h.editor.state().languages().get("z").is_none(),
        "途中の並びは当てない"
    );
    settle(&mut h);
    assert!(h.editor.state().languages().get("z").is_some());
    assert_eq!(h.editor.next_deadline(), None);
}

// verifies: AC-FR-706-2
#[test]
fn 読み直しの最中に監視を止めたら_待たせた自動の読み直しはしない() {
    let (mut h, fake) = start(&Setup {
        files: &[(WS_CONFIG, b"")],
        ..Setup::default()
    });
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[file.watch]\nenabled = false\n[editor]\ntab_width = 2\n",
    );
    h.advance_ms(WATCH_SETTLE.0);
    assert!(h.run_one_job(), "ユーザー層を読んだ");
    // ワークスペース層を読んでいる間に次の書込が来て、期限を過ぎた (順番待ちに積む)
    write(
        &mut h,
        &fake,
        CONFIG,
        b"[file.watch]\nenabled = false\n[editor]\ntab_width = 3\n",
    );
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert_eq!(tab(&h), 2, "止めた後は順番待ちの読み直しもしない");
    assert!(!h.editor.state().config().watch_enabled);
    assert_eq!(h.editor.next_deadline(), None);
    assert_eq!(h.pending_jobs(), 0);
}

// verifies: AC-FR-706-1
#[test]
fn キーマップの誤りの_reload_kept_は同じ内容で_1_回だけ() {
    let (mut h, fake) = start(&Setup::default());
    write(&mut h, &fake, KEYMAP, b"[keys.editor\n");
    settle(&mut h);
    assert_eq!(count(&h, "msg.config.reload_kept"), 1, "{:?}", h.messages());
    h.clear_log();
    // 同じ内容のまま、ほかのファイルの変更で読み直す
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    write(&mut h, &fake, KEYMAP, b"[keys.editor\n");
    settle(&mut h);
    assert_eq!(count(&h, "msg.config.reload_kept"), 0, "{:?}", h.messages());
    assert_eq!(count(&h, "msg.config.problem"), 0, "{:?}", h.messages());
}

// ---------- 段 9 のレビュー: 連鎖の前と同じ診断の要約を出し直さない ----------

/// 手動の `config.reload` で診断を一覧に載せてから、記録を区切る。
fn reload_and_clear(h: &mut Harness) {
    h.cmd("config.reload");
    h.run_jobs();
    assert!(
        h.messages()
            .iter()
            .any(|m| m.starts_with("msg.config.problem")),
        "前提: 手動の読み直しで診断が出る: {:?}",
        h.messages()
    );
    h.clear_log();
}

// verifies: AC-FR-706-1
#[test]
fn テーマファイルの同じ診断は自動の読み直しのたびに出し直さない() {
    let (mut h, fake) = start(&Setup {
        user: "[theme]\nname = \"mine\"\n",
        files: &[(
            "/evo/config/themes/mine.toml",
            b"[ui]\nbogus_role = { bg = \"#112233\" }\n[bogus_table]\nx = 1\n",
        )],
        ..Setup::default()
    });
    reload_and_clear(&mut h);
    for key in ["f5", "f6"] {
        let data = format!("[keys.editor]\n\"{key}\" = \"file.save\"\n");
        write(&mut h, &fake, KEYMAP, data.as_bytes());
        settle(&mut h);
        assert_eq!(h.messages(), vec!["info.config.reloaded"], "{key}");
        h.clear_log();
    }
    // 設定ファイルを同じ内容で保存し直しても出さない
    write(&mut h, &fake, CONFIG, b"[theme]\nname = \"mine\"\n");
    settle(&mut h);
    assert!(h.messages().is_empty(), "{:?}", h.messages());
}

// verifies: AC-FR-706-1
#[test]
fn テーマが無い診断も自動の読み直しのたびに出し直さない() {
    let (mut h, fake) = start(&Setup {
        user: "[theme]\nname = \"nothere\"\n",
        ..Setup::default()
    });
    reload_and_clear(&mut h);
    write(
        &mut h,
        &fake,
        KEYMAP,
        b"[keys.editor]\n\"f5\" = \"file.save\"\n",
    );
    settle(&mut h);
    assert_eq!(h.messages(), vec!["info.config.reloaded"]);
}

// verifies: AC-FR-706-1
#[test]
fn 言語定義の同じ診断も自動の読み直しのたびに出し直さない() {
    let (mut h, fake) = start(&Setup {
        files: &[(
            "/evo/config/languages/y.toml",
            b"[[language]]\nname = \"y\"\nfile_types = [\"yy\"]\nbogus = 1\n",
        )],
        ..Setup::default()
    });
    reload_and_clear(&mut h);
    write(&mut h, &fake, CONFIG, b"");
    settle(&mut h);
    assert!(h.messages().is_empty(), "{:?}", h.messages());
    write(
        &mut h,
        &fake,
        KEYMAP,
        b"[keys.editor]\n\"f5\" = \"file.save\"\n",
    );
    settle(&mut h);
    assert_eq!(h.messages(), vec!["info.config.reloaded"]);
}

// verifies: AC-FR-706-1
#[test]
fn ワークスペース層のキーマップの同じ診断も出し直さない() {
    let (mut h, fake) = start(&Setup {
        files: &[(
            "/w/repo/.eve/keymap.toml",
            b"[keys.editor]\n\"f7\" = \"no.such.command\"\n",
        )],
        ..Setup::default()
    });
    reload_and_clear(&mut h);
    write(&mut h, &fake, CONFIG, b"[editor]\ntab_width = 2\n");
    settle(&mut h);
    assert_eq!(h.messages(), vec!["info.config.reloaded"]);
    // 新しい診断は出す
    write(
        &mut h,
        &fake,
        "/w/repo/.eve/keymap.toml",
        b"[keys.editor]\n\"f8\" = \"no.such.other\"\n",
    );
    settle(&mut h);
    assert!(
        h.messages()
            .iter()
            .any(|m| m.starts_with("msg.config.problem")),
        "{:?}",
        h.messages()
    );
}

// verifies: AC-FR-706-1
#[test]
fn 張り直してすぐ失効するディレクトリでは読み直しを繰り返さない() {
    let (mut h, fake) = start(&Setup::default());
    // 張り直した要求には、すぐ失効が届く (監視の読取がすぐ失敗するディレクトリ)
    let mut reads = 0;
    let mut lost: Vec<WatchId> = Vec::new();
    for _ in 0..5 {
        let id = id_of(&fake, DIR);
        if !lost.contains(&id) {
            lost.push(id);
            fake.push(id, FsEventKind::Lost);
        }
        h.pump_fs();
        h.advance_ms(WATCH_SETTLE.0);
        reads += h.pending_jobs();
        h.run_jobs();
        h.pump_fs();
    }
    let renewals = fake
        .requests()
        .iter()
        .filter(|r| matches!(r, WatchRequest::Watch { dir, .. } if dir.display() == DIR))
        .count();
    assert_eq!(
        renewals,
        2,
        "最初の要求と、1 回の張り直しだけ: {:?}",
        fake.requests()
    );
    assert!(
        reads <= 2,
        "失効の知らせが来なくなれば読み直さない: {reads}"
    );
    // 間隔の後の読み直しで張り直す
    h.advance_ms(eve_core::CONFIG_RENEW_MIN.0);
    h.cmd("config.reload");
    h.run_jobs();
    let renewals = fake
        .requests()
        .iter()
        .filter(|r| matches!(r, WatchRequest::Watch { dir, .. } if dir.display() == DIR))
        .count();
    assert_eq!(renewals, 3);
}
