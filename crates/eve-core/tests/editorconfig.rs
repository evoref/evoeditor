//! `.editorconfig` の読込と重ね方 (docs/07 §10.1・§10.3 の EC-1〜EC-8、§3.6 の LAYER-1 の (5) と LAYER-3 / LAYER-4、
//! 01 FR-705 AC-3 (`.editorconfig` の部分) と AC-4) の core の側。
//!
//! 偽 FS のカレントディレクトリは `/w`。ワークスペースのルートは `--workspace /w/repo` で決め、`/w/other` はルートの外。
//! `.editorconfig` は `/w/repo/.editorconfig` (近い) と `/w/.editorconfig` (遠い) に置く。

use eve_config::{Conditions, Frontend, IndentUnit, Os};
use eve_core::{
    Args, ConfigSources, FsErrorKind, FsPath, InitialFile, JobOutput, JobResult, OwnerVerdict,
    StartupConfig,
};
use eve_testkit::helpers::cell_text;
use eve_testkit::{FsOp, Harness};
use eve_view::OverlayModel;

const CONFIG: &str = "/evo/config/config.toml";
const A: &str = "/w/repo/src/a.rs";
const B: &str = "/w/repo/b.txt";
const OUTSIDE: &str = "/w/other/c.rs";
const EC_REPO: &str = "/w/repo/.editorconfig";
const EC_W: &str = "/w/.editorconfig";
const EC_ROOT: &str = "/.editorconfig";

/// 起動の材料。
#[derive(Default)]
struct Setup<'a> {
    user: &'a str,
    ws: Option<&'a str>,
    sets: &'a [&'a str],
    no_workspace_config: bool,
    /// 置くファイル (パス, 中身)。
    files: &'a [(&'a str, &'a [u8])],
    /// 起動時に開くファイル (空なら `A`)。
    open: &'a [&'a str],
    /// 起動の前に偽 FS に手を加える。
    before: Option<&'a dyn Fn(&Harness)>,
}

fn linux() -> Conditions {
    Conditions {
        os: Some(Os::Linux),
        frontend: Frontend::Tui,
        ..Conditions::default()
    }
}

/// `.editorconfig` の表示名 (Port の組み立てと同じ `join`)。
fn ec(dir: &str) -> String {
    FsPath::from(dir).join(".editorconfig").display()
}

fn sources(s: &Setup<'_>) -> ConfigSources {
    ConfigSources {
        files: vec![("config.toml".to_owned(), FsPath::new(CONFIG))],
        keymap: None,
        sets: s.sets.iter().map(|x| (*x).to_owned()).collect(),
        themes: None,
        languages: None,
        locales: None,
        conditions: linux(),
        no_workspace_config: s.no_workspace_config,
        home: None,
    }
}

fn start(s: &Setup<'_>) -> Harness {
    let mut b = Harness::builder()
        .defer_start()
        .dir("/evo/config")
        .dir("/w/repo/.git")
        .dir("/w/repo/src")
        .dir("/w/other")
        .file(CONFIG, s.user.as_bytes())
        .file(A, b"fn main() {}\n")
        .file(B, b"text\n")
        .file(OUTSIDE, b"fn x() {}\n");
    if let Some(t) = s.ws {
        b = b.file("/w/repo/.eve/config.toml", t.as_bytes());
    }
    for (p, data) in s.files {
        b = b.file(p, data);
    }
    let mut h = b.build();
    if let Some(f) = s.before {
        f(&h);
    }
    let sets: Vec<String> = s.sets.iter().map(|x| (*x).to_owned()).collect();
    let resolved = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(s.user.to_owned())),
        }],
        &sets,
        linux(),
    );
    h.editor.configure(StartupConfig {
        config: Some(resolved.config),
        config_origins: Some(resolved.origins),
        config_user: Some(resolved.user),
        config_languages: Some(resolved.languages),
        config_sources: Some(sources(s)),
        workspace: Some("/w/repo".to_owned()),
        ..Default::default()
    });
    let files = if s.open.is_empty() { &[A][..] } else { s.open };
    let initial: Vec<InitialFile> = files.iter().map(|f| InitialFile::path(*f)).collect();
    h.editor.open_initial_files(&initial, false, None);
    h.started();
    h.run_jobs();
    h
}

fn doc(h: &Harness) -> &eve_core::Document {
    h.editor.state().document(h.doc_id()).expect("文書がある")
}

/// フォーカスの文書の (字下げの単位, 字下げの幅, タブの幅)。
fn indent(h: &Harness) -> (IndentUnit, u8, u8) {
    let d = doc(h);
    (d.indent.unit, d.indent.width, d.display.tab_width)
}

/// 一覧 (`config.show` か `config.show_diagnostics`) を `query` で絞り込んだ (本文, 右の列)。
fn list(h: &mut Harness, command: &str, query: &str) -> Vec<(String, String)> {
    h.cmd(command);
    if h.frame().overlays.is_empty() {
        return Vec::new();
    }
    h.text(query);
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let rows = items
        .iter()
        .map(|i| {
            (
                cell_text(&i.label),
                i.detail.as_ref().map(cell_text).unwrap_or_default(),
            )
        })
        .collect();
    h.cmd("ui.cancel");
    rows
}

fn show(h: &mut Harness, key: &str) -> (String, String) {
    list(h, "config.show", key)
        .into_iter()
        .find(|(l, _)| l.starts_with(&format!("{key} =")))
        .unwrap_or_else(|| panic!("{key} の行が無い"))
}

fn diagnostics(h: &mut Harness) -> Vec<(String, String)> {
    list(h, "config.show_diagnostics", "")
}

/// 区切りを `/` にそろえる (文書のパスは開くときに正規化される)。
fn slash(p: &str) -> String {
    p.replace('\\', "/")
}

fn count(h: &Harness, key: &str) -> usize {
    h.messages().iter().filter(|m| *m == key).count()
}

/// 実行待ちのジョブを 1 件ずつ実行し、`.editorconfig` の読込の結果だけを渡さずに返す (ほかの結果は渡す)。
fn run_holding_editorconfig(h: &mut Harness) -> Vec<JobOutput> {
    let mut held = Vec::new();
    while h.pending_jobs() > 0 {
        for o in h.run_one_job_held() {
            if matches!(o.body, JobResult::EditorConfigRead(_)) {
                held.push(o);
            } else {
                h.deliver(vec![o]);
            }
        }
    }
    held
}

/// `config.reload` を、`.editorconfig` の読込を積むところまで進める (ユーザー層とワークスペース層の読込を 1 件ずつ)。
fn reload_until_editorconfig(h: &mut Harness) {
    h.cmd("config.reload");
    // ReadConfig → ReadWorkspaceConfig (その結果で設定を当て、`.editorconfig` の読込を積む)
    assert!(h.run_one_job());
    assert!(h.run_one_job());
}

// ---- AC-4: 探し方・当たるセクション・優先順位 ----

// verifies: AC-FR-705-4
#[test]
fn 当たるセクションの値が文書に効き_root_で止まり_由来は_editorconfig() {
    let mut h = start(&Setup {
        files: &[
            (EC_ROOT, b"[*]\nindent_size = x\n"),
            (EC_W, b"root = true\n[*]\nindent_size = 8\n"),
            (
                EC_REPO,
                b"# repo\n[*.rs]\nindent_style = tab\nindent_size = 2\ntab_width = 3\n",
            ),
        ],
        ..Setup::default()
    });
    assert_eq!(indent(&h), (IndentUnit::Tab, 2, 3));
    assert_eq!(
        show(&mut h, "editor.indent_width"),
        (
            "editor.indent_width = 2".to_owned(),
            ".editorconfig:4 [*.rs]".to_owned()
        )
    );
    // root = true のファイルより上は読まない (誤った値の診断が無い)
    assert!(diagnostics(&mut h).is_empty(), "{:?}", diagnostics(&mut h));
    // 当たらないセクションは効かず、遠いファイルの値が効く
    h.open(B);
    h.run_jobs();
    assert_eq!(indent(&h), (IndentUnit::Space, 8, 8));
    assert_eq!(show(&mut h, "editor.indent_width").1, ".editorconfig:3 [*]");
    // `Enter` で値を書いたファイルのその行を開く
    h.open(A);
    h.run_jobs();
    h.cmd("config.show");
    h.text("editor.tab_width");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_path().map(|p| slash(&p)), Some(slash(&ec("/w/repo"))));
}

// verifies: AC-FR-705-4
#[test]
fn 優先順位は利用者の範囲より強く_ワークスペース層と_set_より弱い() {
    let mut h = start(&Setup {
        user: "[editor]\nindent_width = 6\ntab_width = 6\n[[language]]\nname = \"rust\"\nindent = { width = 5 }\n",
        ws: Some("[editor]\ntab_width = 7\n"),
        sets: &["editor.indent_unit=space"],
        files: &[(
            EC_W,
            b"[*.rs]\nindent_style = tab\nindent_size = 2\ntab_width = 3\n",
        )],
        open: &[A, OUTSIDE],
        ..Setup::default()
    });
    h.open(A);
    h.run_jobs();
    // (5) は言語定義 (4) と一般の値 (2) に勝ち、ワークスペース層 (6) と --set (8) に負ける
    assert_eq!(indent(&h), (IndentUnit::Space, 2, 7));
    assert_eq!(show(&mut h, "editor.indent_unit").1, "--set");
    assert_eq!(
        show(&mut h, "editor.tab_width").1,
        "ワークスペース: .eve/config.toml:2"
    );
    // ルートの外の文書にも `.editorconfig` は効く (ワークスペース層は効かない、EC-7)
    h.open(OUTSIDE);
    h.run_jobs();
    assert_eq!(indent(&h), (IndentUnit::Space, 2, 3));
    assert_eq!(show(&mut h, "editor.tab_width").1, ".editorconfig:4 [*.rs]");
}

// verifies: AC-FR-705-4
#[test]
fn indent_size_の_tab_は_tab_width_か下の段のタブの幅を使う() {
    let mut h = start(&Setup {
        user: "[editor]\ntab_width = 5\n",
        files: &[(
            EC_REPO,
            b"[*.rs]\nindent_size = tab\n[*.txt]\nindent_size = tab\ntab_width = 3\n",
        )],
        open: &[A, B],
        ..Setup::default()
    });
    h.open(A);
    h.run_jobs();
    assert_eq!(indent(&h), (IndentUnit::Space, 5, 5));
    h.open(B);
    h.run_jobs();
    assert_eq!(indent(&h), (IndentUnit::Space, 3, 3));
}

// ---- LAYER-4: 新しいファイルだけの改行と文字コード ----

// verifies: AC-FR-705-4
#[test]
fn 改行と文字コードは新しいファイルだけに効き_既存のファイルは判定値を保つ() {
    let mut h = start(&Setup {
        files: &[(EC_REPO, b"[*]\nend_of_line = crlf\ncharset = utf-8-bom\n")],
        ..Setup::default()
    });
    let d = doc(&h);
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Lf);
    assert!(!d.format.has_bom);
    h.open("/w/repo/new.rs");
    h.run_jobs();
    let d = doc(&h);
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Crlf);
    assert!(d.format.has_bom);
    assert_eq!(
        show(&mut h, "file.default_line_ending"),
        (
            "file.default_line_ending = \"crlf\"".to_owned(),
            ".editorconfig:2 [*]".to_owned()
        )
    );
}

// verifies: AC-FR-705-4
#[test]
fn 届く前に改行を選んだ新しいファイルと_打った新しいファイルは決め直さない() {
    let mut h = start(&Setup {
        files: &[(EC_REPO, b"[*]\nend_of_line = crlf\ncharset = utf-16le\n")],
        ..Setup::default()
    });
    // 改行を明示的に選んだ (届く前)
    h.open("/w/repo/new1.rs");
    let held = run_holding_editorconfig(&mut h);
    assert_eq!(held.len(), 1);
    h.cmd_with("file.set_line_ending", Args::new().with_str("eol", "lf"));
    h.deliver(held);
    let d = doc(&h);
    assert_eq!(d.format.default_eol, eve_text::LineEnding::Lf);
    assert_eq!(
        d.format.encoding,
        eve_core::Encoding::Utf8,
        "文字コードも決め直さない"
    );
    // 打った (届く前)
    h.open("/w/repo/new2.rs");
    let held = run_holding_editorconfig(&mut h);
    h.text("x");
    h.deliver(held);
    assert_eq!(doc(&h).format.default_eol, eve_text::LineEnding::Lf);
    assert_eq!(h.doc_text(), "x", "本文は変えない");
    // 何もしていなければ届いたときに決める。その後に打ってから読み直しても変えない
    h.open("/w/repo/new3.rs");
    h.run_jobs();
    assert_eq!(doc(&h).format.default_eol, eve_text::LineEnding::Crlf);
    h.text("y");
    h.fs.add_file(EC_REPO, b"[*]\nend_of_line = lf\n");
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(doc(&h).format.default_eol, eve_text::LineEnding::Crlf);
    assert_eq!(doc(&h).format.encoding, eve_core::Encoding::Utf16Le);
}

// ---- 古い結果 ----

// verifies: AC-FR-705-4
#[test]
fn 古い読込の結果は新しい値を上書きしない() {
    let mut h = start(&Setup {
        files: &[(EC_REPO, b"[*]\nindent_size = 2\n")],
        ..Setup::default()
    });
    assert_eq!(indent(&h).1, 2);
    // config.reload の読込を 2 回積み、後の結果を先に渡す
    h.fs.add_file(EC_REPO, b"[*]\nindent_size = 3\n");
    h.cmd("config.reload");
    let first = run_holding_editorconfig(&mut h);
    h.fs.add_file(EC_REPO, b"[*]\nindent_size = 4\n");
    h.cmd("config.reload");
    let second = run_holding_editorconfig(&mut h);
    assert_eq!((first.len(), second.len()), (1, 1));
    assert_eq!(indent(&h).1, 2, "届くまでは前の値");
    h.deliver(second);
    assert_eq!(indent(&h).1, 4);
    h.deliver(first);
    assert_eq!(indent(&h).1, 4, "古い結果は捨てる");
    // 言語を替えても (5) の値は変わらない (パスで決まる)
    h.cmd_with("lang.set", Args::new().with_str("language", "plain"));
    assert_eq!(indent(&h).1, 4);
}

// verifies: AC-FR-705-4
#[test]
fn 名前を付けて保存でパスが変わったら前の結果を捨てて新しいパスで読み直す() {
    let mut h = start(&Setup {
        files: &[
            (EC_REPO, b"[*.rs]\nindent_size = 2\n"),
            (EC_W, b"[*.txt]\nindent_size = 6\n"),
        ],
        ..Setup::default()
    });
    assert_eq!(indent(&h).1, 2);
    h.cmd("config.reload");
    let old = run_holding_editorconfig(&mut h);
    assert_eq!(old.len(), 1);
    h.cmd_with(
        "file.save_as",
        Args::new().with_str("path", "/w/other/a.txt"),
    );
    h.run_jobs();
    assert_eq!(h.doc_path(), Some(FsPath::from("/w/other/a.txt").display()));
    assert_eq!(indent(&h).1, 6, "新しいパス (名前も) で当たるセクション");
    h.deliver(old);
    assert_eq!(indent(&h).1, 6, "前のパスの結果は捨てる");
}

// verifies: AC-FR-705-4
#[test]
fn 閉じた文書と開き直した文書に前の結果を当てない() {
    let mut h = start(&Setup {
        files: &[(EC_REPO, b"[*]\nindent_size = 2\n")],
        ..Setup::default()
    });
    let first = h.doc_id();
    h.fs.add_file(EC_REPO, b"[*]\nindent_size = 3\n");
    h.cmd("config.reload");
    let old = run_holding_editorconfig(&mut h);
    h.cmd("buffer.close");
    h.run_jobs();
    assert!(h.editor.state().document(first).is_none());
    h.fs.add_file(EC_REPO, b"[*]\nindent_size = 5\n");
    h.open(A);
    h.run_jobs();
    assert_ne!(h.doc_id(), first);
    h.deliver(old);
    assert_eq!(indent(&h).1, 5);
}

// ---- 主スレッドで読まない (EC-1・EC-8) ----

// verifies: AC-FR-705-4
#[test]
fn 主スレッドは結果を当てるだけで読込と照合をしない() {
    // EC-4 の資源の上限ぎりぎりの glob (入れ子 8 段・後戻りの多い星) と長いパス
    let mut text = String::from("[*]\nindent_size = 2\n");
    for i in 0..100 {
        text.push_str(&format!(
            "[{}{i}{}]\nindent_size = 3\n[{}]\ntab_width = 4\n",
            "{a,".repeat(8),
            "}".repeat(8),
            "*a".repeat(400)
        ));
    }
    let deep = format!("/w/repo/{}/x.rs", ["d"; 60].join("/"));
    let mut h = start(&Setup {
        files: &[(EC_REPO, text.as_bytes()), (&deep, b"")],
        ..Setup::default()
    });
    h.open(&deep);
    let before = eve_config::editorconfig_work_on_this_thread();
    let held = run_holding_editorconfig(&mut h);
    let after_jobs = eve_config::editorconfig_work_on_this_thread();
    assert!(after_jobs > before, "ジョブの中で読んで照合した");
    h.deliver(held);
    assert_eq!(
        eve_config::editorconfig_work_on_this_thread(),
        after_jobs,
        "結果を当てるときに読み・照合をしない"
    );
    assert_eq!(indent(&h).1, 2);
    // 文書ごとの値を決め直す経路 (言語の切替・config.show) でも読まない
    h.cmd_with("lang.set", Args::new().with_str("language", "plain"));
    show(&mut h, "editor.indent_width");
    assert_eq!(eve_config::editorconfig_work_on_this_thread(), after_jobs);
}

// ---- 失敗の経路 ----

// verifies: AC-FR-705-3
#[test]
fn 所有者の検査で読まなかったファイルは飛ばして上へ続け_その_root_は見ない() {
    let mut h = start(&Setup {
        files: &[
            (EC_REPO, b"root = true\n[*]\nindent_size = 2\n"),
            (EC_W, b"[*]\nindent_size = 4\n"),
        ],
        before: Some(&|h: &Harness| {
            h.fs.set_owner_verdict(
                EC_REPO,
                Some(OwnerVerdict::NotOwner {
                    owner: "mallory".to_owned(),
                }),
            );
            // 他人が書けるディレクトリに `.editorconfig` が無いだけなら診断を出さない (OWN-6 の Missing)
            h.fs.set_owner_verdict(
                "/w/other",
                Some(OwnerVerdict::OthersCanWrite {
                    who: vec!["Everyone".to_owned()],
                }),
            );
        }),
        open: &[A, OUTSIDE],
        ..Setup::default()
    });
    h.open(A);
    h.run_jobs();
    assert_eq!(indent(&h).1, 4);
    let d = diagnostics(&mut h);
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].0.contains("mallory"), "{d:?}");
    assert_eq!(d[0].1, ec("/w/repo"));
    assert_eq!(count(&h, "msg.config.problem"), 1, "{:?}", h.messages());
}

// verifies: AC-FR-705-3
#[test]
fn 安全な場所の下の_editorconfig_は所有者の検査を掛けずに読む() {
    let mut h = start(&Setup {
        user: "[workspace]\nsafe_directories = [\"/w/repo\"]\n",
        files: &[(EC_REPO, b"[*]\nindent_size = 2\n")],
        before: Some(&|h: &Harness| {
            h.fs.set_owner_verdict(
                "/w/repo",
                Some(OwnerVerdict::Unknown {
                    fs: "exFAT".to_owned(),
                }),
            );
        }),
        ..Setup::default()
    });
    assert_eq!(indent(&h).1, 2);
    assert_eq!(diagnostics(&mut h), Vec::<(String, String)>::new());
}

// verifies: AC-FR-705-3
#[test]
fn 読めない_大きすぎる_ファイルは警告して上へ続け_合計の上限で止める() {
    let big = vec![b'#'; 256 * 1024 + 1];
    let mut h = start(&Setup {
        files: &[
            (EC_REPO, b"[*]\nindent_size = \xff\n"),
            ("/w/repo/src/.editorconfig", &big),
            (EC_W, b"[*]\nindent_size = 4\n"),
        ],
        ..Setup::default()
    });
    assert_eq!(indent(&h).1, 4);
    let d = diagnostics(&mut h);
    let texts: Vec<&str> = d.iter().map(|(t, _)| t.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("256 KiB を超えています")),
        "{d:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("UTF-8 ではありません")),
        "{d:?}"
    );

    // 合計 1 MiB: 近い 4 つ (255 KiB ずつ) の後の 5 つ目 (`/w`) で上限を超え、そこから上は使わない
    let mut pad = vec![b'#'; 255 * 1024];
    pad.extend_from_slice(b"\n[*]\ntab_width = 2\n");
    let mut far = pad.clone();
    far.extend_from_slice(b"[*]\nindent_size = 7\n");
    let mut h = start(&Setup {
        files: &[
            ("/w/repo/d1/d2/d3/x.txt", b""),
            ("/w/repo/d1/d2/d3/.editorconfig", &pad),
            ("/w/repo/d1/d2/.editorconfig", &pad),
            ("/w/repo/d1/.editorconfig", &pad),
            (EC_REPO, &pad),
            (EC_W, &far),
        ],
        open: &["/w/repo/d1/d2/d3/x.txt"],
        ..Setup::default()
    });
    h.open("/w/repo/d1/d2/d3/x.txt");
    h.run_jobs();
    assert_eq!(indent(&h).1, 4, "/w の値は使わない");
    let d = diagnostics(&mut h);
    assert!(
        d.iter()
            .any(|(t, f)| t.contains("読む量の上限") && *f == ec("/w")),
        "{d:?}"
    );
}

// verifies: AC-FR-705-3
#[test]
fn 一時の失敗は診断を出さず_読み直しでは前の値を保つ_安全な場所を変えたら外す() {
    let mut h = start(&Setup {
        files: &[(EC_REPO, b"[*]\nindent_size = 2\n")],
        before: Some(&|h: &Harness| h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy)),
        ..Setup::default()
    });
    // 起動時のワークスペース層の読込が失敗を使ったかに依らず、診断もメッセージも無い
    assert_eq!(diagnostics(&mut h), Vec::<(String, String)>::new());
    // 開いたときに一時の失敗: 値は無く、診断を出さない (次の読込で読み直す)
    h.open(B);
    h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy);
    h.run_jobs();
    assert_eq!(indent(&h).1, 4);
    assert_eq!(diagnostics(&mut h), Vec::<(String, String)>::new());
    assert!(
        !h.messages().iter().any(|m| m.starts_with("msg.")),
        "{:?}",
        h.messages()
    );
    // 読み直し (config.reload) で読める
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(indent(&h).1, 2);
    // 読み直しで一時の失敗: 前の値を保ち、知らせる
    reload_until_editorconfig(&mut h);
    h.fs.busy_times(FsOp::ReadChecked, 2);
    h.run_jobs();
    assert_eq!(indent(&h).1, 2);
    assert!(
        count(&h, "msg.config.reload_kept") >= 1,
        "{:?}",
        h.messages()
    );
    // 安全な場所を変えた読み直しで一時の失敗: 前の値を外して知らせる
    h.fs.add_file(CONFIG, b"[workspace]\nsafe_directories = [\"/srv\"]\n");
    reload_until_editorconfig(&mut h);
    h.fs.busy_times(FsOp::ReadChecked, 2);
    h.run_jobs();
    assert_eq!(indent(&h).1, 4);
    assert!(
        count(&h, "msg.workspace.not_reread") >= 1,
        "{:?}",
        h.messages()
    );
}

// ---- 診断 (EC-6) ----

// verifies: AC-FR-705-4
#[test]
fn 同じファイルの同じ中身の診断はセッションで_1_回だけ要約する() {
    let mut h = start(&Setup {
        files: &[(EC_REPO, b"[*]\nindent_size = 0\nmax_line_length = 80\n")],
        open: &[A, B],
        ..Setup::default()
    });
    h.open(B);
    h.run_jobs();
    h.open(A);
    h.run_jobs();
    assert_eq!(count(&h, "msg.config.problem"), 1, "{:?}", h.messages());
    let d = diagnostics(&mut h);
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(
        d[0].0
            .contains(".editorconfig の indent_size = 0 は使えません"),
        "{d:?}"
    );
    assert_eq!(d[0].1, format!("{}:2", ec("/w/repo")));
    assert!(
        d[1].0.contains("max_line_length"),
        "未対応の情報診断: {d:?}"
    );
    // 読み直しでも同じ中身なら要約しない (一覧にも残る)
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(count(&h, "msg.config.problem"), 1);
    assert_eq!(diagnostics(&mut h).len(), 2);
    // 中身が変われば一覧を置き換えて要約する
    h.fs.add_file(EC_REPO, b"[*]\nindent_size = 99\n");
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(count(&h, "msg.config.problem"), 2);
    let d = diagnostics(&mut h);
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].0.contains("indent_size = 99"), "{d:?}");
    // 一覧の Enter でそのファイルのその行を開く
    h.cmd("config.show_diagnostics");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_path().map(|p| slash(&p)), Some(slash(&ec("/w/repo"))));
    // ファイルを消して読み直したら、一覧から外す (最後に読んだときの診断)
    h.fs.remove(&FsPath::from(EC_REPO));
    h.cmd("config.reload");
    h.run_jobs();
    assert!(diagnostics(&mut h).is_empty(), "{:?}", diagnostics(&mut h));
}

// verifies: AC-FR-705-4
#[test]
fn 一つのファイルの診断は上限で切り_要約は省いた分も数える() {
    let mut text = String::from("[*]\n");
    for _ in 0..150 {
        text.push_str("indent_size = 0\n");
    }
    let mut h = start(&Setup {
        files: &[(EC_REPO, text.as_bytes())],
        ..Setup::default()
    });
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let OverlayModel::List { footer, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let footer = cell_text(footer);
    assert!(footer.contains("101"), "100 件と「ほか 50 件」: {footer}");
    h.cmd("ui.cancel");
    assert_eq!(count(&h, "msg.config.problems"), 1, "{:?}", h.messages());
}

// ---- 読まない起動・パスの無い文書 ----

// verifies: AC-FR-705-4
#[test]
fn no_workspace_config_では読まず_無題の文書にも使わない() {
    let mut h = start(&Setup {
        no_workspace_config: true,
        files: &[(EC_REPO, b"[*]\nindent_size = 2\nindent_size = x\n")],
        ..Setup::default()
    });
    assert_eq!(indent(&h).1, 4);
    assert_eq!(diagnostics(&mut h), Vec::<(String, String)>::new());
    let mut h = start(&Setup {
        files: &[(EC_W, b"[*]\nindent_size = 2\n")],
        ..Setup::default()
    });
    h.cmd("file.new");
    h.run_jobs();
    assert!(doc(&h).path.is_none());
    assert_eq!(indent(&h).1, 4);
}

// ---- Windows のパス (EC-1) ----

// verifies: AC-FR-705-4
#[test]
fn 大文字小文字と短い名前で開いても当たり方が変わらない_windows_だけ() {
    let mut h = start(&Setup {
        files: &[
            (
                EC_REPO,
                b"[src/*.rs]\nindent_size = 2\n[sources/*.rs]\nindent_size = 3\n",
            ),
            ("/w/repo/sources/b.rs", b""),
        ],
        before: Some(&|h: &Harness| {
            // 開いたときの名前 → ディスクの上の名前 (MemFs のリンクで canonical のパスを作る)
            h.fs.add_link("/w/repo/Src/a.rs", A);
            h.fs.add_link("/w/repo/SOURCE~1/b.rs", "/w/repo/sources/b.rs");
        }),
        open: &["/w/repo/Src/a.rs"],
        ..Setup::default()
    });
    let (want_src, want_short) = if cfg!(windows) { (2, 3) } else { (4, 4) };
    h.open("/w/repo/Src/a.rs");
    h.run_jobs();
    assert_eq!(
        indent(&h).1,
        want_src,
        "Unix は開いたときのパスのまま (大文字小文字を区別する)"
    );
    h.open("/w/repo/SOURCE~1/b.rs");
    h.run_jobs();
    assert_eq!(indent(&h).1, want_short);
}
