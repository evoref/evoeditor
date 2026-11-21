//! `fuzz_gitignore` の本体 (docs/09 §3.3、03 §12.7.3)。cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_gitignore.rs`) と、
//! Windows の CI で回す proptest (`crates/eve-core/tests/fuzz_gitignore.rs`) が同じ関数を使う (libFuzzer は MSVC 非対応のため、
//! Linux の夜間ジョブだけで fuzz する)。
//!
//! 入力のバイト列を「ignore のファイルの本文 + 照らすパス + ディレクトリか + `core.ignorecase` + 置き場所の深さ」に読み替え、
//! 次を確かめる (git との一致は fuzz ではなく差分試験で見る、09 §3.4):
//!
//! - パニックしない。
//! - 1 回の照合の手数が「各行の (字句の数 + 1) × (パスの長さ + 1) の和」を超えない (`*` の後戻りで時間が爆発しない)。
//! - 同じ入力で結果が決定的 (解釈し直しても同じ)。
//! - 置き場所を 1 段下げて、パスもその分を外して照らしても結果が同じ。連鎖 (`ExcludeChain`) を通しても結果が同じ。
//! - ワイルドカード・エスケープ・否定を含まない行は、名前かパスの完全一致にだけ当たる (行を 1 つずつ別に確かめる)。
//! - 文字の集合 (`[...]`) を含まない本文では、`core.ignorecase` が真のときの結果は、本文とパスの ASCII の英字を小文字にして
//!   から偽で照らした結果と同じ。
//! - 速い経路で照らす行 (`**/` + ワイルドカードの無い名前) の結果は、同じ行を字句の模擬で照らした結果と同じ。

use super::{CaseFold, ExcludeChain, IgnoreRules, PathKind, RepoIgnore};

/// 読み替えた入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitignoreCase {
    /// ignore のファイルの本文。
    pub text: Vec<u8>,
    /// 照らすパス (`/` 区切りの相対パス。空の要素を含まない)。
    pub path: Vec<u8>,
    /// 項目の種類。
    pub kind: PathKind,
    /// `core.ignorecase`。
    pub fold: CaseFold,
    /// 置き場所をパスの先頭の何要素にするか (0 なら最上位)。
    pub base_depth: usize,
}

/// libFuzzer の入口。性質に反したらパニックする。
///
/// # Panics
///
/// 性質に反したとき (fuzz のクラッシュ)。
pub fn gitignore(data: &[u8]) {
    if let Err(e) = check(&decode(data)) {
        panic!("{e}");
    }
}

/// 入力のバイト列を読み替える。先頭の 1 バイトが種類・畳み方・置き場所の深さ、次の 1 バイトが本文とパスの境目 (残りの
/// 長さに対する割合)。足りないバイトは 0 として読む。
#[must_use]
pub fn decode(data: &[u8]) -> GitignoreCase {
    let flags = data.first().copied().unwrap_or(0);
    let split = data.get(1).copied().unwrap_or(0);
    let rest = data.get(2..).unwrap_or(&[]);
    let at = rest.len() * usize::from(split) / 256;
    let (text, raw_path) = rest.split_at(at);
    let segments: Vec<&[u8]> = raw_path
        .split(|b| *b == b'/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut path = segments.join(&b'/');
    for b in &mut path {
        if *b == 0 {
            *b = b'x';
        }
    }
    if path.is_empty() {
        path = b"a".to_vec();
    }
    let depth = path.iter().filter(|b| **b == b'/').count();
    GitignoreCase {
        text: text.to_vec(),
        path,
        kind: if flags & 1 == 0 {
            PathKind::File
        } else {
            PathKind::Dir
        },
        fold: if flags & 2 == 0 {
            CaseFold::Sensitive
        } else {
            CaseFold::Ascii
        },
        base_depth: usize::from(flags >> 2) % (depth + 1),
    }
}

/// 性質を確かめる。
///
/// # Errors
///
/// 性質に反したとき、その説明。
pub fn check(c: &GitignoreCase) -> Result<(), String> {
    let base_len = nth_slash(&c.path, c.base_depth);
    let base = &c.path[..base_len];
    let (rules, _) = IgnoreRules::parse(base, &c.text);
    let mut steps = 0;
    let got = rules.decide_counted(&c.path, c.kind, c.fold, &mut steps);
    let bound = rules.step_bound(c.path.len());
    if steps > bound {
        return Err(format!("照合の手数 {steps} が上限 {bound} を超えた: {c:?}"));
    }
    // 決定的
    let (again, _) = IgnoreRules::parse(base, &c.text);
    let mut steps2 = 0;
    if again.decide_counted(&c.path, c.kind, c.fold, &mut steps2) != got || steps2 != steps {
        return Err(format!("同じ入力で結果が変わった: {c:?}"));
    }
    // 置き場所を 1 段下げても同じ
    if c.base_depth > 0 {
        let first = nth_slash(&c.path, 1);
        let shifted_base = if base_len > first {
            &c.path[first + 1..base_len]
        } else {
            &[]
        };
        let shifted_path = &c.path[first + 1..];
        let (shifted, _) = IgnoreRules::parse(shifted_base, &c.text);
        let mut s = 0;
        if shifted.decide_counted(shifted_path, c.kind, c.fold, &mut s) != got {
            return Err(format!("置き場所をずらすと結果が変わった: {c:?}"));
        }
    }
    // 連鎖を通しても同じ (`info/exclude` として)
    let (chain, _) = ExcludeChain::default().enter_repo(RepoIgnore::new(rules.clone(), c.fold));
    let mut chain_steps = 0;
    let via_chain = chain
        .decide_counted(&c.path, c.kind, &mut chain_steps)
        .map(|h| h.excluded);
    if via_chain != got.map(|(_, e)| e) {
        return Err(format!("連鎖を通すと結果が変わった: {c:?}"));
    }
    let chain_bound = chain.step_bound(c.path.len());
    if chain_steps > chain_bound {
        return Err(format!(
            "連鎖の照合の手数 {chain_steps} が上限 {chain_bound} を超えた: {c:?}"
        ));
    }
    if let Some(line) = rules.fast_path_mismatch(&c.path, c.fold) {
        return Err(format!(
            "{line} 行目の速い経路の結果が字句の模擬と違う: {c:?}"
        ));
    }
    check_literal_lines(c, base)?;
    check_fold(c, base, got.map(|(_, e)| e))
}

/// `n` 個目 (1 始まり) の `/` の位置。`n` が 0 なら 0、足りなければパスの長さ。
fn nth_slash(path: &[u8], n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    path.iter()
        .enumerate()
        .filter(|(_, b)| **b == b'/')
        .nth(n - 1)
        .map_or(path.len(), |(i, _)| i)
}

/// ワイルドカード・エスケープ・否定を含まない行は、名前かパスの完全一致にだけ当たる。
fn check_literal_lines(c: &GitignoreCase, base: &[u8]) -> Result<(), String> {
    let rel = if base.is_empty() {
        &c.path[..]
    } else {
        &c.path[base.len() + 1..]
    };
    let name = rel.rsplit(|b| *b == b'/').next().unwrap_or(rel);
    let body = c.text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&c.text);
    for raw in body.split(|b| *b == b'\n') {
        let mut line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if let Some(nul) = line.iter().position(|b| *b == 0) {
            line = &line[..nul];
        }
        if raw.is_empty()
            || raw[0] == b'#'
            || raw.len() > super::IGNORE_LINE_MAX_BYTES
            || line.first() == Some(&b'!')
            || line.iter().any(|b| matches!(b, b'*' | b'?' | b'[' | b'\\'))
        {
            continue;
        }
        let mut pat = line;
        while let [rest @ .., b' '] = pat {
            pat = rest;
        }
        let dir_only = pat.last() == Some(&b'/');
        if dir_only {
            pat = &pat[..pat.len() - 1];
        }
        let anchored = pat.contains(&b'/');
        if anchored && pat.first() == Some(&b'/') {
            pat = &pat[1..];
        }
        let subject = if anchored { rel } else { name };
        let eq = match c.fold {
            CaseFold::Sensitive => pat == subject,
            CaseFold::Ascii => pat.eq_ignore_ascii_case(subject),
        };
        let want = !pat.is_empty() && eq && (!dir_only || c.kind == PathKind::Dir);
        let (one, _) = IgnoreRules::parse(base, line);
        let got = one.decide(&c.path, c.kind, c.fold).is_some();
        if got != want {
            return Err(format!(
                "ワイルドカードの無い行 {:?} の一致が完全一致と違う (期待 {want}): {c:?}",
                String::from_utf8_lossy(line)
            ));
        }
    }
    Ok(())
}

/// 文字の集合を含まない本文では、畳む照合と「小文字にしてから区別する照合」が同じ。
fn check_fold(c: &GitignoreCase, base: &[u8], got: Option<bool>) -> Result<(), String> {
    if c.fold != CaseFold::Ascii || c.text.contains(&b'[') {
        return Ok(());
    }
    let lower_text = c.text.to_ascii_lowercase();
    let lower_path = c.path.to_ascii_lowercase();
    let lower_base = &lower_path[..base.len()];
    let (lowered, _) = IgnoreRules::parse(lower_base, &lower_text);
    let want = lowered
        .decide(&lower_path, c.kind, CaseFold::Sensitive)
        .map(|h| h.excluded);
    if want != got {
        return Err(format!(
            "畳む照合 ({got:?}) が小文字にしてからの照合 ({want:?}) と違う: {c:?}"
        ));
    }
    Ok(())
}
