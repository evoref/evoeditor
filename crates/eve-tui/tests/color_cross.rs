//! 色の近似 (04 §12.3) の交差試験 (docs/review/v0.3-findings.md の STR-E4)。
//!
//! 同じ近似が 2 か所にある: テーマファイルの 256 / 16 の段を求める eve-config (`theme_file`) と、Frame の色を端末の段に
//! 収める最終防衛線の eve-tui (`color`)。依存の向き (02 §4.1。eve-tui は eve-config に依存しない) で関数を共有できないので、
//! ここで同じ入力に同じ結果を返すことを確かめる。

use eve_config::theme_file as cfg;
use eve_tui::color as tui;

/// 確かめる RGB の各軸の値: 0〜255 を 15 刻みと、6×6×6 キューブの値とその前後 (境目で取り違えやすい)。
fn axis() -> Vec<u8> {
    let mut v: Vec<u8> = (0..=255u8).step_by(15).collect();
    for c in [0u8, 95, 135, 175, 215, 255] {
        v.extend([c.saturating_sub(1), c, c.saturating_add(1)]);
    }
    v.sort_unstable();
    v.dedup();
    v
}

#[test]
fn xterm_の番号と_rgb_は同じ() {
    for i in 0..=255u8 {
        assert_eq!(cfg::xterm_rgb(i), tui::xterm_rgb(i), "{i}");
    }
}

#[test]
fn rgb_の_256_色と_16_色への近似は同じ() {
    let axis = axis();
    // 全組合せは多いので、2 軸を全部・残りの 1 軸を間引いた 3 通りの面で見る
    for &a in &axis {
        for &b in &axis {
            for &c in axis.iter().step_by(4) {
                for rgb in [(a, b, c), (a, c, b), (c, a, b)] {
                    assert_eq!(
                        cfg::rgb_to_ansi256(rgb),
                        tui::rgb_to_ansi256(rgb),
                        "{rgb:?}"
                    );
                    assert_eq!(cfg::rgb_to_ansi16(rgb), tui::rgb_to_ansi16(rgb), "{rgb:?}");
                }
            }
        }
    }
}
