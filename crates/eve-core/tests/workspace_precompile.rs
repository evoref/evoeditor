//! ワークスペース層の言語定義の `first_line` の正規表現を、主スレッドで組まないこと (docs/07 §9.6 の段 2 の細目、§12.5 の
//! WSC-2。M33 の段 5 のレビュー)。
//!
//! 組んだ回数はプロセスで 1 つの数 (`eve_config::first_line_compiles`) なので、ほかの試験と並べて走らせないよう、この試験
//! だけをこのファイルに置く。Harness はジョブを試験のスレッドで走らせるので、ジョブを走らせる所 (`run_one_job_held`) と
//! 結果を主スレッドへ渡す所 (`deliver`) を分け、渡す所の前後で数が変わらないことを確かめる。

use eve_config::{Conditions, Frontend, Os};
use eve_core::{ConfigSources, FsPath, InitialFile, StartupConfig};
use eve_testkit::Harness;

// verifies: AC-FR-704-2
#[test]
fn ワークスペース層の言語定義の正規表現は_主スレッドに渡すどの結果でも組まない() {
    // `^` で始まらない (行頭の文字列で前もって外せない) 正規表現を 20 個
    let ws: String = (0..20)
        .map(|i| format!("[[language]]\nname = \"l{i}\"\nfirst_line = \"zz{i}\\\\d+\"\n"))
        .collect();
    let mut h = Harness::builder()
        .defer_start()
        .dir("/evo/config")
        .dir("/w/repo/.git")
        .file("/evo/config/config.toml", b"")
        .file("/w/repo/.eve/config.toml", ws.as_bytes())
        .file("/w/repo/a.unknown", b"text\n")
        .file("/w/repo/b.unknown", b"text\n")
        .build();
    let conditions = Conditions {
        os: Some(Os::Linux),
        frontend: Frontend::Tui,
        ..Conditions::default()
    };
    h.editor.configure(StartupConfig {
        config_sources: Some(ConfigSources {
            files: vec![(
                "config.toml".to_owned(),
                FsPath::new("/evo/config/config.toml"),
            )],
            conditions,
            ..ConfigSources::default()
        }),
        workspace: Some("/w/repo".to_owned()),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path("/w/repo/a.unknown")], false, None);
    h.started();
    let start = eve_config::first_line_compiles();
    for _ in 0..1_000 {
        let outs = h.run_one_job_held();
        if outs.is_empty() {
            break;
        }
        let before = eve_config::first_line_compiles();
        h.deliver(outs);
        assert_eq!(
            eve_config::first_line_compiles(),
            before,
            "主スレッドで結果を受けて正規表現を組んだ"
        );
    }
    // ワークスペース層の 20 個はジョブで組んだ (この試験の経路を通った)
    assert!(
        eve_config::first_line_compiles() >= start + 20,
        "{} → {}",
        start,
        eve_config::first_line_compiles()
    );
    // ワークスペース層が届いた後も、文書の判定 (`lang.set auto` で判定し直す) で組まない
    let before = eve_config::first_line_compiles();
    auto(&mut h);
    assert_eq!(eve_config::first_line_compiles(), before);

    // config.reload: ジョブを 1 件ずつ渡すたびに判定し直しても組まない。ワークスペース層が届いた後に、まだ開いていない文書を
    // 開いても (開くジョブは言語定義の読込より後に並ぶ) 組まない
    h.cmd("config.reload");
    let mut opened = false;
    for _ in 0..1_000 {
        let outs = h.run_one_job_held();
        if outs.is_empty() {
            break;
        }
        let workspace_read = outs
            .iter()
            .any(|o| matches!(o.body, eve_core::JobResult::WorkspaceConfigRead(_)));
        let before = eve_config::first_line_compiles();
        h.deliver(outs);
        auto(&mut h);
        if workspace_read && !opened {
            h.open("/w/repo/b.unknown");
            opened = true;
        }
        assert_eq!(
            eve_config::first_line_compiles(),
            before,
            "config.reload の後に主スレッドで正規表現を組んだ"
        );
    }
    assert!(opened, "config.reload でワークスペース層を読み直した");
    assert_eq!(
        h.doc_path().as_deref(),
        Some("/w/repo/b.unknown"),
        "まだ開いていない文書を開いた"
    );
}

/// フォーカスの文書の言語を判定し直す (`lang.set` の `auto`)。
fn auto(h: &mut Harness) {
    h.cmd_with(
        "lang.set",
        eve_core::Args::new().with_str("language", "auto"),
    );
}
