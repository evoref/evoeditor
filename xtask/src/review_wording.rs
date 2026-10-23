//! `cargo xtask review-wording`: 日本語 UI 文言の人手レビュー用の一覧を作る (docs/11 §8、09 §9.2 の文言の確認)。
//!
//! `locales/ja.toml` の全文言を、種類 (コマンド・ヒント・確認ダイアログ・メッセージ・状態表示・その他) ごとの表にして
//! `docs/review/ja-wording-v0.1.md` に書く。表は「キー / 文言 / 出す場面 / 判定」で、出す場面は 04 §15.1・§15.2 の目録の
//! 「出す場面」(または「内容」) 列から、キーが一致するものを引く (目録に無いキーは空)。判定の列は、レビューする人が書く。
//! 文言の変更はこの一覧に書いた指摘を受けて別の PR で行う (この生成物は `ja.toml` から作り直せるので、手では直さない)。

use std::collections::BTreeMap;

use eve_docs_tables::{identifiers, read_tables};

use crate::util::{read_text, workspace_root};

const HEADINGS: [&str; 2] = [
    "### 15.1 v0.1 の画面文言 (ja)",
    "### 15.2 保存・読込・退避・パニックの文言目録",
];
const SCENE_COLUMNS: [&str; 3] = ["出す場面", "出す場面 (07 §4.2)", "内容"];
const OUT: &str = "docs/review/ja-wording-v0.1.md";

fn flatten(prefix: &str, v: &toml::Value, out: &mut BTreeMap<String, String>) {
    match v {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
        }
        _ => {}
    }
}

/// 目録のキー → 出す場面。`msg.<x>` の略記は `.text` / `.hint` の両方に同じ場面を引く。
fn scenes(doc04: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for h in HEADINGS {
        let Ok(tables) = read_tables(doc04, h, &["MessageKey"]) else {
            continue;
        };
        for t in &tables {
            let Some(col) = SCENE_COLUMNS.iter().find(|c| t.column(c).is_some()) else {
                continue;
            };
            for row in &t.rows {
                let scene = t.cell(row, col).unwrap_or_default().trim().to_owned();
                for key in identifiers(t.cell(row, "MessageKey").unwrap_or_default()) {
                    out.entry(key.clone()).or_insert_with(|| scene.clone());
                    if key.starts_with("msg.") && !key.ends_with(".text") && !key.ends_with(".hint")
                    {
                        out.entry(format!("{key}.text"))
                            .or_insert_with(|| scene.clone());
                        out.entry(format!("{key}.hint"))
                            .or_insert_with(|| scene.clone());
                    }
                }
            }
        }
    }
    out
}

fn group_of(key: &str) -> (&'static str, u8) {
    match key.split('.').next().unwrap_or("") {
        "cmd" => ("コマンド (パレットの名前と説明)", 0),
        "hint" => ("ヒントバーのキー表示 (8 セル以内)", 1),
        "ui" if key.starts_with("ui.confirm.") => ("確認ダイアログ", 2),
        "ui" if key.starts_with("ui.prompt.") => ("入力プロンプト", 3),
        "ui" => ("画面の文言 (検索バー・パレット・その他)", 4),
        "msg" => ("メッセージ: 失敗・警告 (何が + どうする)", 5),
        "info" => ("メッセージ: 通知", 6),
        "status" => ("ステータスバー", 7),
        _ => ("その他", 8),
    }
}

/// 表のセルに入れる形にする (`|` と改行)。
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', "<br>")
}

pub fn run() -> Result<(), String> {
    let root = workspace_root();
    let value: toml::Value = toml::from_str(&read_text(&root.join("locales/ja.toml"))?)
        .map_err(|e| format!("locales/ja.toml を読めません: {e}"))?;
    let mut texts = BTreeMap::new();
    flatten("", &value, &mut texts);
    texts.retain(|k, _| !k.starts_with("meta."));
    let scenes = scenes(&read_text(&root.join("docs/04-tui-design.md"))?);

    let mut groups: BTreeMap<(u8, &'static str), Vec<(&String, &String)>> = BTreeMap::new();
    for (k, v) in &texts {
        let (name, order) = group_of(k);
        groups.entry((order, name)).or_default().push((k, v));
    }

    let mut md = String::new();
    md.push_str("# 日本語 UI 文言のレビュー一覧 (v0.1)\n\n");
    md.push_str(
        "`cargo xtask review-wording` が `locales/ja.toml` から生成した一覧です。手では直しません \
         (文言を変えるときは `locales/ja.toml` と docs/04 §15 を直し、この一覧は作り直します)。\n\n",
    );
    md.push_str(&format!("文言は全部で {} 個です。\n\n", texts.len()));
    md.push_str(
        "**レビューの観点** (docs/11 §8、NFR-602): (1) 失敗のメッセージは「何が起きたか」と「どうすればよいか」が分かる。\
         (2) 用語が統一されている (保存・書き込み、開く・読み込む、退避・バックアップ、など)。\
         (3) 文末と敬体が揃っている (です・ます)。(4) 角括弧のキー表記 (`[S]` など) が実際のキーと合っている。\
         (5) 引数の `{…}` に入る値で文が崩れない (長いパス・空の値)。(6) ヒントバーの表示は 8 セル以内。\n\n",
    );
    md.push_str("判定の列には `OK` か、直し方を書いてください。\n\n");
    for ((_, name), rows) in &groups {
        md.push_str(&format!("## {name}\n\n{} 個\n\n", rows.len()));
        md.push_str("| キー | 文言 | 出す場面 | 判定 |\n|---|---|---|---|\n");
        for (k, v) in rows {
            md.push_str(&format!(
                "| `{k}` | {} | {} | |\n",
                cell(v),
                cell(scenes.get(*k).map_or("", String::as_str))
            ));
        }
        md.push('\n');
    }
    let path = root.join(OUT);
    std::fs::create_dir_all(path.parent().expect("docs/review/")).map_err(|e| e.to_string())?;
    std::fs::write(&path, md).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("review-wording: {} 個 → {OUT}", texts.len());
    Ok(())
}
