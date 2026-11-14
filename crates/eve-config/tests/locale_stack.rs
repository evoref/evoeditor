//! ユーザーの言語ファイルの読込が、1 MiB のスタック (Windows のメインスレッドの既定。bin は設定と同じくメインスレッドで
//! 言語ファイルを読む、02 §9.5 の手順 2) で深い入れ子のファイルを読んでも、スタックを使い切らないこと (docs/07 §17.4、
//! 09 §3.4 の (1))。TOML パーサの入れ子の上限 (80) は、インラインの表・配列の入れ子と点のキーの段数を別々に数えるので、
//! 組み合わせると数千段の入れ子を作れる。言語ファイルはインラインの表と配列を構文を読む前に拒む。

// 1 MiB のスタックのスレッドで読むための試験のスレッド (R11 の例外。src の外の試験だけで使う)
#![allow(clippy::disallowed_methods)]

use eve_config::{
    Catalog, DiagKind, LOCALE_MAX_DIAGNOSTICS, LocaleProblem, Severity, load_locale_file,
};

/// 1 MiB のスタックのスレッドで `f` を走らせる (スタックを使い切ると、プロセスごと異常終了して試験が失敗する)。
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(f)
        .expect("試験のスレッドを作れる")
        .join()
        .expect("試験のスレッドが終わる")
}

fn dotted(n: usize) -> String {
    vec!["a"; n].join(".")
}

// verifies: NFR-603
#[test]
fn インラインの表と点のキーを組み合わせた入れ子は読む前に拒む() {
    // `{ a.a.…(79 段) = { a.a.…(79 段) = … } }` を 60 段。パーサの上限に掛からず、約 4,800 段の入れ子になる形
    let n = 60;
    let src = format!(
        "x = {}\"v\"{}\n",
        format!("{{ {} = ", dotted(79)).repeat(n),
        " }".repeat(n)
    );
    let f = on_small_stack(move || {
        load_locale_file(
            "ja",
            src.as_bytes(),
            Catalog::builtin_ja(),
            LOCALE_MAX_DIAGNOSTICS,
        )
    });
    assert_eq!(f.diagnostics().len(), 1, "{:?}", f.diagnostics());
    let d = &f.diagnostics()[0];
    assert_eq!(d.severity, Severity::Error);
    assert_eq!(d.kind, DiagKind::Locale(LocaleProblem::InlineValue));
    assert_eq!((d.line, d.column), (1, 5));
    assert!(f.texts().is_empty());
}

// verifies: NFR-603
#[test]
fn 段数の多いキーは読む前に拒み_書ける深さのファイルは_1_mib_のスタックで読める() {
    // 表の見出しと点のキーの、それぞれパーサの上限より 1 段浅い段数 (パーサは通すが、合わせて 157 段になる)
    let deep = format!("[{}]\n{} = \"x\"\n", dotted(78), dotted(79));
    let f = on_small_stack(move || {
        load_locale_file(
            "ja",
            deep.as_bytes(),
            Catalog::builtin_ja(),
            LOCALE_MAX_DIAGNOSTICS,
        )
    });
    assert_eq!(f.diagnostics().len(), 1);
    assert_eq!(
        f.diagnostics()[0].kind,
        DiagKind::Locale(LocaleProblem::TooDeep)
    );
    // 書ける最も深いキー (見出しと点のキーで 8 段) を数多く並べたファイル
    let mut src = String::new();
    for i in 0..2000 {
        src.push_str(&format!("[{}.t{i}]\n{} = \"x\"\n", dotted(4), dotted(3)));
    }
    let f = on_small_stack(move || {
        load_locale_file("ja", src.as_bytes(), Catalog::builtin_ja(), usize::MAX)
    });
    assert_eq!(f.diagnostics().len(), 2000, "{:?}", f.diagnostics().first());
    assert!(
        f.diagnostics()
            .iter()
            .all(|d| d.kind == DiagKind::Locale(LocaleProblem::UnknownKey))
    );
}
