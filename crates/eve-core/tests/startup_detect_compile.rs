//! 名前・拡張子で言語が決まる起動時の文書の判定で、`first_line` の正規表現を組まないこと (docs/07 §9.6 の段 2 の細目。
//! M33 の段 5 の再レビュー)。
//!
//! 組んだ回数はプロセスで 1 つの数 (`eve_config::first_line_compiles`) なので、ほかの試験と並べて走らせないよう、この試験
//! だけをこのファイルに置く。

use eve_config::{Conditions, Frontend, Os};
use eve_core::{ConfigSources, FsPath, InitialFile, StartupConfig};
use eve_testkit::Harness;

// verifies: AC-FR-704-2
#[test]
fn 拡張子で決まる起動時の文書の判定では_ユーザー層の先頭行の正規表現を組まない() {
    // `^` で始まらない (行頭の文字列で前もって外せない) ユーザー層の正規表現
    let user = "[[language]]\nname = \"u\"\nfirst_line = \"uu\\\\d+\"\n";
    let mut h = Harness::builder()
        .defer_start()
        .dir("/evo/config")
        .file("/evo/config/config.toml", user.as_bytes())
        .file("/w/a.rs", b"fn main() {}\n")
        .build();
    let resolved = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(user.to_owned())),
        }],
        &[],
        Conditions::default(),
    );
    h.editor.configure(StartupConfig {
        config_languages: Some(resolved.languages),
        config_sources: Some(ConfigSources {
            files: vec![(
                "config.toml".to_owned(),
                FsPath::new("/evo/config/config.toml"),
            )],
            conditions: Conditions {
                os: Some(Os::Linux),
                frontend: Frontend::Tui,
                ..Conditions::default()
            },
            ..ConfigSources::default()
        }),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.rs")], false, None);
    h.started();
    for _ in 0..1_000 {
        // 区切って譲るジョブ (`.editorconfig` の探索など) の 1 区切りは結果を送らないので、待ちの数で終わりを見る
        if h.pending_jobs() == 0 {
            break;
        }
        let outs = h.run_one_job_held();
        let before = eve_config::first_line_compiles();
        h.deliver(outs);
        assert_eq!(
            eve_config::first_line_compiles(),
            before,
            "主スレッドで結果を受けて正規表現を組んだ"
        );
    }
    let lang = h.editor.state().context_value("doc.language");
    assert!(
        matches!(&lang, Some(eve_core::CtxValue::Str(s)) if s == "rust"),
        "{lang:?}"
    );
}
