//! `cargo xtask gen-notices`: `THIRD-PARTY-NOTICES.md` の生成 (docs/05 §17.1、docs/11 §9.6)。
//!
//! `cargo about` (`about.toml` と `about.hbs`) で、配布する exe の依存の著作権表示とライセンスを列挙する。
//! `--check` は、生成した結果が今のファイルと一致することだけを確かめる (書き換えない。依存を更新した PR で
//! 再生成を忘れていないかの検査)。`cargo-about` が無ければ、入れ方を示して失敗する。
//! crate ではない静的リンクの部品 (Linux の静的版に入る musl libc) の表示は、`dist/notices/` の原文を末尾に足す。

use std::process::Command;

use crate::util::{read_text, run_capture, workspace_root};

const OUT: &str = "THIRD-PARTY-NOTICES.md";

/// crate ではない部品の表示 (見出し, 原文のファイル)。原文は配布元のものをそのまま置く。
const EXTRA: &[(&str, &str)] = &[(
    "musl libc 1.2.5 (Linux の musl 静的版に静的リンク。MIT License)",
    "dist/notices/musl-COPYRIGHT",
)];

pub fn run(args: &[String]) -> Result<(), String> {
    let check = args.iter().any(|a| a == "--check");
    let root = workspace_root();
    let tmp = root.join("target/gen-notices.md");
    std::fs::create_dir_all(tmp.parent().expect("target/ の下")).map_err(|e| e.to_string())?;
    run_capture(
        Command::new("cargo")
            .args([
                "about",
                "generate",
                "--locked",
                "--manifest-path",
                "bins/evoeditor/Cargo.toml",
                "-o",
            ])
            .arg(&tmp)
            .arg("about.hbs"),
    )
    .map_err(|e| {
        format!(
            "{e}\ncargo-about が要ります: cargo install --locked cargo-about@0.9.2 --features cli"
        )
    })?;
    let mut generated = read_text(&tmp)?;
    for (title, src) in EXTRA {
        let text = read_text(&root.join(src))?;
        generated.push_str(&format!(
            "## {title}

```text
{}
```
",
            text.trim_end()
        ));
    }
    let path = root.join(OUT);
    if check {
        let current = read_text(&path).unwrap_or_default();
        if current != generated {
            return Err(format!(
                "{OUT} が依存と一致しません。cargo xtask gen-notices で再生成してください"
            ));
        }
        println!("gen-notices: {OUT} は最新です");
    } else {
        std::fs::write(&path, generated).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("gen-notices: {OUT} を書きました");
    }
    Ok(())
}
