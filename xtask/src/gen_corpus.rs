//! `cargo xtask gen-corpus`: 巨大・長大のコーパスを決定的に生成する (docs/09-testing-quality.md §4)。
//!
//! 生成物はコミットしない (`testdata/generated/` は `.gitignore` で除外)。同じ `--seed` なら、ベンチ・テスト双方で
//! 同じ内容になる。内容はすべて合成したデータ (手書きの語句の組み合わせ) で、実在の文書は使わない (OQ-168、09 §4)。
//!
//! 生成する種類 (1 MB = 1,048,576 バイト):
//! - `large-1mb` / `large-10mb` / `large-100mb` / `large-1gb`: 平均行長 80 バイトの文書 (NFR-104 の条件)。LF 区切り
//! - `longline-ascii-10mb` / `longline-japanese-10mb` / `longline-emoji-10mb`: 10 MB の 1 行 (NFR-106 AC-3 の 3 種)
//! - `many-lines-1m`: 100 万行の短い行
//!
//! 既定では `large-1gb` を除く (時間と容量が大きい)。`--with-1gb` で加える。

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::util::workspace_root;

const MIB: u64 = 1024 * 1024;

/// 生成する種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 平均行長 80 バイトの文書。バイト数の目標を持つ。
    Large(u64),
    /// 1 行の長大行。内容の種類とバイト数の目標を持つ。
    LongLine(Flavor, u64),
    /// 短い行を並べた文書。行数を持つ。
    ManyLines(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Ascii,
    Japanese,
    Emoji,
}

const CATALOG: [(&str, Kind); 8] = [
    ("large-1mb", Kind::Large(MIB)),
    ("large-10mb", Kind::Large(10 * MIB)),
    ("large-100mb", Kind::Large(100 * MIB)),
    ("large-1gb", Kind::Large(1024 * MIB)),
    (
        "longline-ascii-10mb",
        Kind::LongLine(Flavor::Ascii, 10 * MIB),
    ),
    (
        "longline-japanese-10mb",
        Kind::LongLine(Flavor::Japanese, 10 * MIB),
    ),
    (
        "longline-emoji-10mb",
        Kind::LongLine(Flavor::Emoji, 10 * MIB),
    ),
    ("many-lines-1m", Kind::ManyLines(1_000_000)),
];

// ---------- 決定的な乱数 (splitmix64。外部依存なし) ----------

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 0 以上 `n` 未満。
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

// ---------- 合成した語句 ----------

const ASCII_WORDS: [&str; 24] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango",
    "uniform", "victor", "whiskey", "yankee",
];

const JA_PHRASES: [&str; 16] = [
    "これは試験用の文章です",
    "吾輩は猫である",
    "テキストエディタの動作を確かめる",
    "東京都千代田区",
    "全角ＡＢＣと半角abc",
    "ひらがなとカタカナと漢字",
    "ｶﾀｶﾅの半角",
    "長い行の表示を確認する",
    "保存と読み込みの往復",
    "文字コードの判定",
    "改行コードの保持",
    "検索と置換の速度",
    "あいうえお、かきくけこ。",
    "「括弧」と（括弧）",
    "一二三四五六七八九十",
    "ＡＢＣ１２３",
];

const EMOJI_MIX: [&str; 12] = [
    "😀",
    "👨\u{200D}👩\u{200D}👧",
    "🇯🇵",
    "👍🏽",
    "か\u{3099}",
    "e\u{0301}",
    "※",
    "○",
    "α",
    "漢字",
    "ascii",
    "🎌",
];

// ---------- 生成 ----------

/// 平均行長 80 バイトの文書を、`target` バイトを超える最初の行の終わりまで書く。
fn write_large(w: &mut impl Write, rng: &mut Rng, target: u64) -> io::Result<u64> {
    let mut written = 0u64;
    let mut line = String::with_capacity(160);
    while written < target {
        line.clear();
        // 行ごとの目標を 28〜108 バイトにし、語句を足してその長さを超えたら止める (平均が約 80 バイトになる)
        let goal = 28 + rng.below(81);
        while line.len() < goal {
            if !line.is_empty() {
                line.push(' ');
            }
            if rng.below(3) == 0 {
                line.push_str(rng.pick(&JA_PHRASES));
            } else {
                line.push_str(rng.pick(&ASCII_WORDS));
            }
        }
        line.push('\n');
        w.write_all(line.as_bytes())?;
        written += line.len() as u64;
    }
    Ok(written)
}

/// 改行を含まない 1 行を、おおよそ `target` バイト (超えない最大。マルチバイトの途中では切らない) 書く。末尾に LF を 1 つ付ける。
fn write_long_line(
    w: &mut impl Write,
    rng: &mut Rng,
    flavor: Flavor,
    target: u64,
) -> io::Result<u64> {
    let mut written = 0u64;
    let mut first = true;
    loop {
        let piece: &str = match flavor {
            Flavor::Ascii => rng.pick(&ASCII_WORDS),
            Flavor::Japanese => rng.pick(&JA_PHRASES),
            Flavor::Emoji => rng.pick(&EMOJI_MIX),
        };
        let sep = if first || flavor == Flavor::Japanese {
            ""
        } else {
            " "
        };
        let add = (sep.len() + piece.len()) as u64;
        if written + add > target {
            // 残りを ASCII の 'x' で埋めて、目標のバイト数ちょうどにする
            let rest = target - written;
            w.write_all(&vec![b'x'; rest as usize])?;
            written += rest;
            break;
        }
        w.write_all(sep.as_bytes())?;
        w.write_all(piece.as_bytes())?;
        written += add;
        first = false;
    }
    w.write_all(b"\n")?;
    Ok(written + 1)
}

/// 短い行を `lines` 行書く。
fn write_many_lines(w: &mut impl Write, rng: &mut Rng, lines: u64) -> io::Result<u64> {
    let mut written = 0u64;
    for i in 0..lines {
        let mut line = format!("{i}");
        match rng.below(3) {
            0 => {
                line.push(' ');
                line.push_str(rng.pick(&ASCII_WORDS));
            }
            1 => {
                line.push(' ');
                line.push_str(rng.pick(&JA_PHRASES));
            }
            _ => {}
        }
        line.push('\n');
        w.write_all(line.as_bytes())?;
        written += line.len() as u64;
    }
    Ok(written)
}

fn generate(kind: Kind, seed: u64, w: &mut impl Write) -> io::Result<u64> {
    let mut rng = Rng(seed);
    match kind {
        Kind::Large(bytes) => write_large(w, &mut rng, bytes),
        Kind::LongLine(flavor, bytes) => write_long_line(w, &mut rng, flavor, bytes),
        Kind::ManyLines(lines) => write_many_lines(w, &mut rng, lines),
    }
}

// ---------- コマンド ----------

struct Options {
    seed: u64,
    out: PathBuf,
    only: Option<Vec<String>>,
    with_1gb: bool,
    list: bool,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        seed: 42,
        out: workspace_root().join("testdata").join("generated"),
        only: None,
        with_1gb: false,
        list: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} には値が要ります"))
        };
        match a.as_str() {
            "--seed" => {
                o.seed = value("--seed")?
                    .parse()
                    .map_err(|e| format!("--seed: {e}"))?
            }
            "--out" => o.out = PathBuf::from(value("--out")?),
            "--only" => o.only = Some(value("--only")?.split(',').map(str::to_owned).collect()),
            "--with-1gb" => o.with_1gb = true,
            "--list" => o.list = true,
            other => return Err(format!("知らないオプションです: {other}")),
        }
    }
    Ok(o)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let o = parse_args(args)?;
    if o.list {
        for (name, _) in CATALOG {
            println!("{name}");
        }
        return Ok(());
    }
    if let Some(only) = &o.only
        && let Some(bad) = only.iter().find(|n| !CATALOG.iter().any(|(c, _)| c == n))
    {
        return Err(format!("知らない名前です: {bad} (--list で一覧)"));
    }
    fs::create_dir_all(&o.out).map_err(|e| format!("{} を作れません: {e}", o.out.display()))?;
    println!("▶ gen-corpus (seed {}、出力 {})", o.seed, o.out.display());
    for (name, kind) in CATALOG {
        let wanted = match &o.only {
            Some(only) => only.iter().any(|n| n == name),
            None => name != "large-1gb" || o.with_1gb,
        };
        if !wanted {
            continue;
        }
        let path = o.out.join(format!("{name}.txt"));
        let bytes =
            write_file(&path, kind, o.seed).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("  {name}.txt  {bytes} バイト");
    }
    Ok(())
}

fn write_file(path: &Path, kind: Kind, seed: u64) -> io::Result<u64> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let n = generate(kind, seed, &mut w)?;
    w.flush()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_bytes(kind: Kind, seed: u64) -> Vec<u8> {
        let mut v = Vec::new();
        generate(kind, seed, &mut v).unwrap();
        v
    }

    #[test]
    fn 同じ_seed_なら同じ内容になる() {
        let a = gen_bytes(Kind::Large(64 * 1024), 42);
        let b = gen_bytes(Kind::Large(64 * 1024), 42);
        assert_eq!(a, b);
        assert_ne!(a, gen_bytes(Kind::Large(64 * 1024), 43));
    }

    #[test]
    fn 平均行長は約_80_バイト() {
        let v = gen_bytes(Kind::Large(MIB), 42);
        let lines = v.iter().filter(|&&b| b == b'\n').count();
        let mean = v.len() as f64 / lines as f64;
        assert!((77.0..=83.0).contains(&mean), "平均行長 {mean}");
        assert!(String::from_utf8(v).is_ok(), "UTF-8 であること");
    }

    #[test]
    fn 長大行は目標のバイト数ちょうどの_1_行() {
        for flavor in [Flavor::Ascii, Flavor::Japanese, Flavor::Emoji] {
            let target = 256 * 1024;
            let v = gen_bytes(Kind::LongLine(flavor, target), 42);
            assert_eq!(v.len() as u64, target + 1, "{flavor:?}: 本文 + LF");
            assert_eq!(
                v.iter().filter(|&&b| b == b'\n').count(),
                1,
                "{flavor:?}: 改行は末尾の 1 つだけ"
            );
            assert!(String::from_utf8(v).is_ok(), "{flavor:?}: UTF-8 であること");
        }
    }

    #[test]
    fn 長大行の絵文字混在は書記素を壊さない() {
        let v = String::from_utf8(gen_bytes(Kind::LongLine(Flavor::Emoji, 64 * 1024), 42)).unwrap();
        assert!(v.contains('😀') || v.contains('🇯'), "絵文字を含むこと");
    }

    #[test]
    fn 短い行の行数は指定どおり() {
        let v = gen_bytes(Kind::ManyLines(1000), 42);
        assert_eq!(v.iter().filter(|&&b| b == b'\n').count(), 1000);
    }
}
