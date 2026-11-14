//! ユーザーの言語ファイルの発見と CLI (docs/07 §17.4、01 NFR-603 AC-9、09 §3.4 の (3))。`cli_config.rs` と同じく、
//! 実バイナリを普通の子プロセスとして起動し、試験用の設定ディレクトリ (`EVE_CONFIG_DIR`) に置いた `locales/` で、
//! 名前の規則・`--locale` と `EVE_LANG` の選択と見つからないときの診断・`ui.locale` の照合・`--no-config`・通常のファイルで
//! ないもの・`--check-config` の出力 (先頭の選択の結果の行・無害化・終了コード)・`--list-commands --json` の `locale` を見る。

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_evoeditor");

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("evo-cli-locale-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("locales")).unwrap();
    d
}

/// 設定ディレクトリを `cfg` にして起動し、(終了コード, 標準出力)。`EVE_LANG` は `lang`。
fn run_env(cfg: &Path, lang: &str, args: &[&str]) -> (i32, String) {
    let out = Command::new(EXE)
        .args(args)
        .env("EVE_CONFIG_DIR", cfg)
        .env("EVE_LANG", lang)
        .output()
        .expect("起動できる");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn run(cfg: &Path, args: &[&str]) -> (i32, String) {
    run_env(cfg, "ja", args)
}

fn write(cfg: &Path, name: &str, text: &str) {
    std::fs::write(cfg.join("locales").join(name), text).unwrap();
}

/// `--list-commands --json` の `locale` と、`file.save` のタイトル。
fn listing(cfg: &Path, lang: &str, args: &[&str]) -> (String, String) {
    let mut all = vec!["--list-commands", "--json"];
    all.extend_from_slice(args);
    let (code, out) = run_env(cfg, lang, &all);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let title = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "file.save")
        .map(|c| c["title"].as_str().unwrap().to_owned())
        .unwrap();
    (v["locale"].as_str().unwrap().to_owned(), title)
}

// verifies: NFR-603
#[test]
fn 追加言語のファイルを見つけ_locale_と_eve_lang_で選べる() {
    let d = dir("pick");
    write(&d, "fr.toml", "[cmd.file.save]\ntitle = \"Enregistrer\"\n");
    // --locale (地域付きも)
    assert_eq!(
        listing(&d, "ja", &["--locale", "fr-FR"]),
        ("fr".to_owned(), "Enregistrer".to_owned())
    );
    // EVE_LANG
    assert_eq!(
        listing(&d, "fr", &[]),
        ("fr".to_owned(), "Enregistrer".to_owned())
    );
    // ui.locale
    std::fs::write(d.join("config.toml"), "[ui]\nlocale = \"fr\"\n").unwrap();
    assert_eq!(
        listing(&d, "en", &[]),
        ("fr".to_owned(), "Enregistrer".to_owned())
    );
    // 書いていないキーは補う言語 en の文言
    let (code, out) = run(&d, &["--list-commands", "--locale", "fr"]);
    assert_eq!(code, 0);
    assert!(out.contains("file.open\tOpen"), "{out}");
    // --no-config では locales/ を見ない (同梱の表だけ)
    std::fs::remove_file(d.join("config.toml")).unwrap();
    assert_eq!(
        listing(&d, "ja", &["--no-config", "--locale", "en"]),
        ("en".to_owned(), "Save".to_owned())
    );
    // 次の段は OS の言語 (試験を回す機械で ja か en) なので、fr でないことだけを見る
    let (locale, _) = listing(&d, "fr", &["--no-config"]);
    assert!(
        locale == "ja" || locale == "en",
        "-N では fr は見つからず、次の段へ進む: {locale}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn 同梱の言語のファイルは同じ言語に重ねる() {
    let d = dir("ja");
    write(&d, "ja.toml", "[cmd.file.save]\ntitle = \"ほぞん\"\n");
    assert_eq!(
        listing(&d, "ja", &[]),
        ("ja".to_owned(), "ほぞん".to_owned())
    );
    // ほかの言語を選べば使わない
    assert_eq!(
        listing(&d, "ja", &["--locale", "en"]),
        ("en".to_owned(), "Save".to_owned())
    );
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn 規則に合わない名前は読まずに警告する() {
    let d = dir("names");
    for name in ["FR.toml", "zh-TW.toml", "con.toml", "pt_BR.TOML"] {
        write(&d, name, "[cmd.file.save]\ntitle = \"X\"\n");
    }
    write(&d, "notes.txt", "not a language file");
    let (code, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 0, "警告だけなら 0: {out}");
    for name in ["FR.toml", "zh-TW.toml", "con.toml", "pt_BR.TOML"] {
        assert!(
            out.contains(&format!(
                "locales/: {name} は言語ファイルの名前の規則に合わないため読みません"
            )),
            "{name}: {out}"
        );
    }
    assert!(!out.contains("notes.txt"), "{out}");
    // FR.toml を fr として読まない
    assert!(
        out.contains("--locale: 言語 fr の言語ファイルがありません"),
        "{out}"
    );
    let (locale, title) = listing(&d, "ja", &["--locale", "fr"]);
    assert_eq!((locale.as_str(), title.as_str()), ("ja", "保存"));
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn 見つからない言語は探した場所と代わりの言語を示す() {
    let d = dir("missing");
    // --locale と EVE_LANG がどちらも見つからないと OS の言語の段へ進む。文言はその言語 (試験を回す機械で ja か en)
    let (code, out) = run_env(&d, "de_DE", &["--check-config", "--locale", "xx"]);
    assert_eq!(code, 0, "{out}");
    let locales = std::path::absolute(d.join("locales")).unwrap();
    if out.lines().next().unwrap().starts_with("言語: ") {
        assert!(
            out.contains("--locale: 言語 xx の言語ファイルがありません"),
            "{out}"
        );
        // 探した場所は --check-config (と一覧) の対処の後にだけ出す
        assert!(
            out.contains(&format!(
                "  locales/ に置くか ja / en を指定 (探した場所: {})",
                locales.display()
            )),
            "{out}"
        );
        assert!(out.contains("(いまは ja)"), "{out}");
        assert!(
            out.contains("EVE_LANG: 言語 de_DE の言語ファイルがありません"),
            "{out}"
        );
    } else {
        assert!(
            out.contains("--locale: There is no language file for xx (en is used)"),
            "{out}"
        );
        assert!(
            out.contains(&format!(
                "  Add it to locales/ or use ja or en (searched: {})",
                locales.display()
            )),
            "{out}"
        );
        assert!(
            out.contains("EVE_LANG: There is no language file for de_DE (en is used)"),
            "{out}"
        );
    }
    // 形に合わない値からもパスを組み立てない
    let (_, out) = run(&d, &["--check-config", "--locale", "../x"]);
    assert!(
        out.contains("--locale: 言語 ../x の言語ファイルがありません"),
        "{out}"
    );
    // --no-config では探していないので、場所を示さない別の文
    let (_, out) = run(&d, &["--check-config", "--no-config", "--locale", "fr"]);
    assert!(
        out.contains("--locale: 言語 fr の言語ファイルがありません (いまは ja)\n  言語ファイルを読まない起動です\n"),
        "{out}"
    );
    assert!(!out.contains("探した場所"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn eve_lang_の_codeset_と_modifier_は落として当てる() {
    let d = dir("posix");
    write(&d, "fr.toml", "[cmd.file.save]\ntitle = \"Enregistrer\"\n");
    assert_eq!(listing(&d, "en_US.UTF-8", &[]).0, "en");
    assert_eq!(listing(&d, "fr_FR.UTF-8@euro", &[]).0, "fr");
    let (_, out) = run_env(&d, "en_US.UTF-8", &["--check-config"]);
    assert!(!out.contains("EVE_LANG:"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn ui_locale_は使える言語と照らす() {
    let d = dir("ui-locale");
    write(&d, "fr.toml", "");
    std::fs::write(d.join("config.toml"), "[ui]\nlocale = \"de\"\n").unwrap();
    // --locale が先に当たっても、ui.locale の照合は行う
    let (code, out) = run(&d, &["--check-config", "--locale", "ja"]);
    assert_eq!(code, 0, "{out}");
    // 値の形に合う言語が見つからなければ、見つからない言語の診断 (書いた行で、探した場所を示す)
    assert!(
        out.contains("config.toml:2:1: 言語 de の言語ファイルがありません (いまは ja)"),
        "{out}"
    );
    assert!(out.contains("(探した場所: "), "{out}");
    // 値の形に合わない値は使えない値 (許容値に候補の言語)
    std::fs::write(d.join("config.toml"), "[ui]\nlocale = \"../x\"\n").unwrap();
    let (_, out) = run(&d, &["--check-config", "--locale", "ja"]);
    assert!(
        out.contains("使える値: auto, ja, en, fr。この項目は既定値 auto を使います"),
        "{out}"
    );
    // 候補にある言語なら知らせない
    std::fs::write(d.join("config.toml"), "[ui]\nlocale = \"fr\"\n").unwrap();
    let (_, out) = run(&d, &["--check-config"]);
    assert!(!out.contains("The value"), "{out}");
    assert!(out.starts_with("Language: fr (ui.locale)"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
// verifies: NFR-503
#[test]
fn check_config_は先頭に選択の結果を出し_言語ファイルを検査する() {
    let d = dir("check");
    write(
        &d,
        "fr.toml",
        "[meta]\nlanguage = \"fr\"\n\
         [cmd.file.save]\ntitle = \"Enregistrer\"\n\
         [no.such]\nkey = \"x\"\n\
         [status]\nuntitled = \"\\u001B]0;pwned\\u0007-{n}\"\n\
         [\"bad\\u001Bkey\"]\nx = \"y\"\n",
    );
    let (code, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 0, "キーごとの問題は警告: {out}");
    let file = std::path::absolute(d.join("locales").join("fr.toml")).unwrap();
    let first = out.lines().next().unwrap();
    // 補う言語 en の同梱の文言で出す
    assert_eq!(
        first,
        format!(
            "Language: fr (--locale), language file: {}, fallback: en",
            file.display()
        )
    );
    assert!(
        out.contains("locales/fr.toml:6:1: There is no text key named no.such.key"),
        "{out}"
    );
    assert!(
        out.contains("locales/fr.toml:8:1: The text of status.untitled contains a character that cannot be used (control character <U+001B>)"),
        "{out}"
    );
    // キーの名前に入れた制御文字も、出力の全体も無害化する
    assert!(out.contains("bad<U+001B>key.x"), "{out}");
    assert!(
        !out.chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r'),
        "{out:?}"
    );
    // ファイル全体の問題はエラーで、終了コード 3
    write(&d, "fr.toml", "[cmd.file.save]\ntitle = [\"x\"]\n");
    let (code, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 3, "{out}");
    assert!(
        out.contains("locales/fr.toml:2:9: This language file is not used because it is not valid: inline tables and arrays cannot be used"),
        "{out}"
    );
    // 言語ファイルの無い言語は 〔 〕 の部分を省く
    let (_, out) = run(&d, &["--check-config", "--locale", "en"]);
    assert_eq!(
        out.lines().next().unwrap(),
        "Language: en (--locale), fallback: en"
    );
    let (_, out) = run(&d, &["--check-config", "--no-config"]);
    assert!(
        out.lines()
            .next()
            .unwrap()
            .starts_with("言語: ja (EVE_LANG)"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn 大きすぎるファイルと_utf8_でないファイルは全体を使わない() {
    let d = dir("whole");
    let mut big = String::from("[cmd.file.save]\ntitle = \"Enregistrer\"\n");
    big.push_str(&"#".repeat(1024 * 1024));
    write(&d, "fr.toml", &big);
    let (code, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 3, "{out}");
    assert!(
        out.contains(
            "locales/fr.toml: This language file is not used because it is too large (over 1 MiB)"
        ),
        "{out}"
    );
    // 全体を使わない: 補う言語 en の文言
    assert_eq!(listing(&d, "ja", &["--locale", "fr"]).1, "Save");
    std::fs::write(d.join("locales").join("fr.toml"), b"title = \"\xff\"\n").unwrap();
    let (code, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("it is not UTF-8"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// `run` を期限付きで行う (止まったら子を終わらせて失敗にする)。出力は小さいので、終わってから読む。
fn run_within(cfg: &Path, args: &[&str]) -> (i32, String) {
    use std::io::Read as _;
    let mut child = Command::new(EXE)
        .args(args)
        .env("EVE_CONFIG_DIR", cfg)
        .env("EVE_LANG", "ja")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("起動できる");
    let t0 = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().expect("待てる") {
            break s;
        }
        if t0.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("止まらずに終わるはず: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    (status.code().unwrap_or(-1), out)
}

// verifies: NFR-603
#[test]
fn ディレクトリを言語ファイルの名前で置いても止まらない() {
    let d = dir("not-regular");
    std::fs::create_dir_all(d.join("locales").join("fr.toml")).unwrap();
    let (code, out) = run_within(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 3, "{out}");
    assert!(
        out.contains("locales/fr.toml: This language file is not used because it cannot be read: it is not a regular file"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

// FIFO は書き手がいなくても開くときに止まらず、通常のファイルでないとして読まない (07 §17.4。Unix は O_NONBLOCK で開く)
// verifies: NFR-603
#[cfg(unix)]
#[test]
fn fifo_を言語ファイルの名前で置いても止まらない() {
    let d = dir("fifo");
    let fifo = d.join("locales").join("fr.toml");
    let st = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
    assert!(st.success());
    let (code, out) = run_within(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("it is not a regular file"), "{out}");
    let (code, out) = run_within(&d, &["--list-commands", "--locale", "fr"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("file.save	Save"), "補う言語 en の文言: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

// verifies: NFR-603
#[test]
fn check_config_は追加言語の訳の無いキーを出し_全件を数える() {
    let d = dir("untranslated");
    // 同梱に無いキーを 150 件: 画面では 100 件と「ほか 50 件」、--check-config では全件
    let mut src = String::from("[cmd.file.save]\ntitle = \"Enregistrer\"\n[zz]\n");
    for i in 0..150 {
        src.push_str(&format!("k{i} = \"x\"\n"));
    }
    write(&d, "fr.toml", &src);
    let (code, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out.matches("There is no text key named zz.k").count(),
        150,
        "{out}"
    );
    assert!(!out.contains("more problems"), "{out}");
    assert!(out.contains("150 problem(s) in the settings."), "{out}");
    // 訳の無いキーの件数と一覧 (情報。件数の行には数えない)
    assert!(
        out.contains("keys have no translation (shown in English)"),
        "{out}"
    );
    assert!(
        out.contains("locales/fr.toml: cmd.file.open.title has no translation"),
        "{out}"
    );
    assert!(
        !out.contains("cmd.file.save.title has no translation"),
        "{out}"
    );
    // 同梱の言語の部分の上書きでは出さない
    write(&d, "ja.toml", "[cmd.file.save]\ntitle = \"ほぞん\"\n");
    let (_, out) = run(&d, &["--check-config", "--locale", "ja"]);
    assert!(!out.contains("の訳がありません"), "{out}");
    // ファイル全体を使っていないときは先頭の行にそう添える
    write(&d, "fr.toml", "title = [\"x\"]\n");
    let (_, out) = run(&d, &["--check-config", "--locale", "fr"]);
    assert!(
        out.lines()
            .next()
            .unwrap()
            .contains("fr.toml (not used), fallback: en"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
