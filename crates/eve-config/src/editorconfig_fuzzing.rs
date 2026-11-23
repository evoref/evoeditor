//! `fuzz_editorconfig` の本体 (docs/09 §3.3、07 §10.3 の EC-3 / EC-4)。cargo-fuzz のターゲット
//! (`fuzz/fuzz_targets/fuzz_editorconfig.rs`) と、Windows の CI で回す proptest (`crates/eve-config/tests/fuzz_editorconfig.rs`)
//! が同じ関数を使う (libFuzzer は MSVC 非対応のため、Linux の夜間ジョブだけで fuzz する。`fuzz_gitignore` と同じ形)。
//!
//! 入力のバイト列を「`.editorconfig` の本文 + 照らすパス」に読み替え、次を確かめる:
//!
//! - パニックしない。読んだセクションは 1,000 個まで (EC-3)。
//! - 各セクションの glob の状態の数が glob の長さ + 4 (先頭に足す `**/` を含む) を超えず、照合の手数が
//!   [`Glob::step_bound`] (状態の数 × パスの長さに比例) に収まる (EC-4。`*` の後戻り・選択肢の展開で時間が爆発しない)。
//! - 同じ入力で結果が決定的 (読み直しても同じファイル・同じ値)。
//! - 重ねた結果の由来は、どれも本文の行の範囲の中を指す。
//! - `{a,b}` の照合は、選択肢を展開した glob のどれかへの照合と一致する (入力から作った、選択肢が上限以下の glob で)。
//! - 資源の上限を超える glob のセクションは、そのセクションを使わないだけで、ほかのセクションの結果を変えない
//!   (そのセクションの見出しを、どこにも当たらない glob に書き換えた本文と同じ結果)。

use crate::editorconfig::{
    EDITORCONFIG_FILE_MAX_SECTIONS, parse_editorconfig, resolve_editorconfig_counted,
};
use crate::glob::{Glob, GlobCase};

/// 読み替えた入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfigCase {
    /// `.editorconfig` の本文 (UTF-8 でない並びは置き換え文字にして読む)。
    pub text: String,
    /// 照らすパス (`.editorconfig` のあるディレクトリからの相対パス。`/` 区切り、空の要素と NUL を含まない)。
    pub path: String,
}

/// libFuzzer の入口。性質に反したらパニックする。
///
/// # Panics
///
/// 性質に反したとき (fuzz のクラッシュ)。
pub fn editorconfig(data: &[u8]) {
    if let Err(e) = check(&decode(data)) {
        panic!("{e}");
    }
}

/// 入力のバイト列を読み替える。先頭の 1 バイトが本文とパスの境目 (残りの長さに対する割合)。足りないバイトは 0 として読む。
#[must_use]
pub fn decode(data: &[u8]) -> EditorConfigCase {
    let split = data.first().copied().unwrap_or(0);
    let rest = data.get(1..).unwrap_or(&[]);
    let at = rest.len() * usize::from(split) / 256;
    let (text, raw_path) = rest.split_at(at);
    // NUL は照らすパスに現れない (書き換えた見出しの glob が当たらないように)
    let path = String::from_utf8_lossy(raw_path).replace(char::from(0), "x");
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let path = if segments.is_empty() {
        "a".to_owned()
    } else {
        segments.join("/")
    };
    EditorConfigCase {
        text: String::from_utf8_lossy(text).into_owned(),
        path,
    }
}

/// 性質を確かめる。
///
/// # Errors
///
/// 性質に反したとき、その説明。
pub fn check(c: &EditorConfigCase) -> Result<(), String> {
    let file = parse_editorconfig(&c.text);
    if file.section_count() > EDITORCONFIG_FILE_MAX_SECTIONS {
        return Err(format!("セクションが上限を超えた: {c:?}"));
    }
    if parse_editorconfig(&c.text) != file {
        return Err(format!("同じ入力で結果が変わった: {c:?}"));
    }
    let chars = c.path.chars().count();
    let mut bound = 0usize;
    for s in &file.sections {
        let Some(g) = &s.glob else {
            continue;
        };
        let limit = s.pattern.chars().count() + 4;
        if g.state_count() > limit {
            return Err(format!(
                "glob {:?} の状態の数 {} が長さ + 4 ({limit}) を超えた",
                s.pattern,
                g.state_count()
            ));
        }
        let mut steps = 0;
        let _ = g.is_match_counted(&c.path, &mut steps);
        let b = g.step_bound(chars);
        if steps > b {
            return Err(format!(
                "glob {:?} の照合の手数 {steps} が上限 {b} を超えた: {c:?}",
                s.pattern
            ));
        }
        bound = bound.saturating_add(b);
    }
    let sources = [("/x/.editorconfig", &file, c.path.as_str())];
    let mut steps = 0;
    let values = resolve_editorconfig_counted(&sources, &mut steps);
    if steps > bound {
        return Err(format!("重ねる手数 {steps} が上限 {bound} を超えた: {c:?}"));
    }
    let mut again = 0;
    if resolve_editorconfig_counted(&sources, &mut again) != values || again != steps {
        return Err(format!("同じ入力で重ねた結果が変わった: {c:?}"));
    }
    let lines = c.text.split('\n').count();
    let origins = [
        values.indent_unit.as_ref().map(|v| &v.1),
        values.indent_width.as_ref().map(|v| &v.1),
        values.tab_width.as_ref().map(|v| &v.1),
        values.line_ending.as_ref().map(|v| &v.1),
        values.encoding.as_ref().map(|v| &v.1),
    ];
    for o in origins.into_iter().flatten() {
        if o.line == 0 || o.line as usize > lines || o.column == 0 {
            return Err(format!("由来 {o:?} が本文の外を指す: {c:?}"));
        }
    }
    check_bad_sections(c, &values)?;
    check_alternatives(c)
}

/// 資源の上限を超える glob のセクションは、そのセクションを使わないだけ: その見出しを当たらない glob に書き換えても結果が同じ。
fn check_bad_sections(
    c: &EditorConfigCase,
    values: &crate::EditorConfigValues,
) -> Result<(), String> {
    let file = parse_editorconfig(&c.text);
    let bad: Vec<u32> = file
        .diagnostics()
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                crate::DiagKind::EditorConfig(crate::EditorConfigProblem::BadGlob(_))
            )
        })
        .map(|d| d.line)
        .collect();
    if bad.is_empty() {
        return Ok(());
    }
    // 当たらない glob (`/` を含み、置き場所の外を指す形)。行の番号を変えないよう、1 行を 1 行に書き換える
    let replaced: Vec<String> = c
        .text
        .split('\n')
        .enumerate()
        .map(|(i, l)| {
            let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
            if bad.contains(&n) {
                "[/\u{0}/never]".to_owned()
            } else {
                l.to_owned()
            }
        })
        .collect();
    let other = parse_editorconfig(&replaced.join("\n"));
    let mut steps = 0;
    let got =
        resolve_editorconfig_counted(&[("/x/.editorconfig", &other, c.path.as_str())], &mut steps);
    if &got != values {
        return Err(format!(
            "使わないセクションがほかのセクションの結果を変えた: {c:?}"
        ));
    }
    Ok(())
}

/// 選択肢の照合は、展開した glob のどれかへの照合と一致する。本文のバイトから、`*`・`{`・`[`・`\` を含まない部品で
/// `a<前>{<選択肢>,…}<後>` を作る (先頭の `/` (置き場所の印) と `**` (区切りとの並びで意味が変わる) を作らない)。
fn check_alternatives(c: &EditorConfigCase) -> Result<(), String> {
    const ALPHABET: [char; 5] = ['a', 'b', '?', '/', '.'];
    let mut pieces: Vec<String> = vec![String::new()];
    for b in c.text.bytes().take(64) {
        if b % 7 == 0 && pieces.len() < 6 {
            pieces.push(String::new());
        } else if let Some(p) = pieces.last_mut() {
            p.push(ALPHABET[usize::from(b) % ALPHABET.len()]);
        }
    }
    if pieces.len() < 4 {
        return Ok(());
    }
    let pre = format!("a{}", pieces[0]);
    let post = pieces[pieces.len() - 1].clone();
    let alts = &pieces[1..pieces.len() - 1];
    let whole = format!("{pre}{{{}}}{post}", alts.join(","));
    let g = Glob::new(&whole, GlobCase::Sensitive).map_err(|e| format!("{whole}: {e:?}"))?;
    let got = g.is_match(&c.path);
    let mut any = false;
    for a in alts {
        let e = Glob::new(&format!("{pre}{a}{post}"), GlobCase::Sensitive)
            .map_err(|e| format!("{pre}{a}{post}: {e:?}"))?;
        any |= e.is_match(&c.path);
    }
    if got != any {
        return Err(format!(
            "{whole} の照合 ({got}) が展開した glob の照合 ({any}) と違う: {}",
            c.path
        ));
    }
    Ok(())
}
