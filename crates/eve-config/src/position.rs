//! 本文のバイト位置から (行, 桁) を引く表と、1 つのファイルの診断の数の上限 (docs/07 §4.2・§12.5 の WSC-2)。
//!
//! 位置を引くたびに先頭から数えると、診断の多いファイル (他人のリポジトリの 256 KiB の配列など) で O(n²) になり、`io` の
//! ジョブを止める。行の始まりの表を 1 回作り、二分探索で行を、行の始まりからの文字数で桁を求める。

/// 1 つのファイルの診断の数の上限 (ユーザー層・ワークスペース層の設定ファイルと `keymap.toml`)。超えた分は残さず、
/// 「ほか N 件」(`DiagKind::More`) の 1 件にまとめる (言語ファイルの `LOCALE_MAX_DIAGNOSTICS` と同じ形、07 §4.3)。
pub const CONFIG_MAX_DIAGNOSTICS: usize = 100;

/// 行の始まりのバイト位置の表。
#[derive(Debug, Clone)]
pub(crate) struct LineIndex<'a> {
    text: &'a str,
    /// 各行の始まりのバイト位置 (先頭は 0)。
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(text: &'a str) -> LineIndex<'a> {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        LineIndex { text, starts }
    }

    /// バイト位置 → (行, 桁)。どちらも 1 始まり、桁は文字数。u32 を超える行・桁は上限で丸める (表示用)。
    pub(crate) fn pos(&self, offset: usize) -> (u32, u32) {
        let end = offset.min(self.text.len());
        let line = self.starts.partition_point(|s| *s <= end);
        let start = self.starts[line.saturating_sub(1)];
        let col = self.text.get(start..end).map_or(0, |l| l.chars().count()) + 1;
        (
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(col).unwrap_or(u32::MAX),
        )
    }
}

/// 1 つのファイルの診断の列に上限を掛ける: 「ほか N 件」(`DiagKind::More`) を除いた診断を
/// [`CONFIG_MAX_DIAGNOSTICS`] 件までにし、超えた分と、元からあった「ほか N 件」の件数を合わせて、最後に 1 件の
/// 「ほか N 件」にする (読込の診断に分類・検証・言語定義の診断を足した後の、ファイルごとの合計に掛ける、07 §4.3)。
/// 残すのは、エラー → ワークスペース層の知らせ (保留・無視・効かない・読まなかった理由・使わない割当) → ほか (未知の
/// キーなど) の順で選び (先頭に未知のキーを並べて保留の知らせを隠せない)、残した診断は元の並びのまま。
pub fn cap_diagnostics(diags: &mut Vec<crate::Diagnostic>) {
    let rank = |d: &crate::Diagnostic| {
        if d.severity == crate::Severity::Error {
            0
        } else if matches!(d.kind, crate::DiagKind::Workspace(_)) {
            1
        } else {
            2
        }
    };
    let mut omitted = 0;
    let mut items: Vec<(usize, crate::Diagnostic)> = Vec::with_capacity(diags.len());
    for (i, d) in diags.drain(..).enumerate() {
        match d.kind {
            crate::DiagKind::More { count } => omitted += count,
            _ => items.push((i, d)),
        }
    }
    if items.len() > CONFIG_MAX_DIAGNOSTICS {
        items.sort_by_key(|(i, d)| (rank(d), *i));
        omitted += items.len() - CONFIG_MAX_DIAGNOSTICS;
        items.truncate(CONFIG_MAX_DIAGNOSTICS);
        items.sort_by_key(|(i, _)| *i);
    }
    let mut kept: Vec<crate::Diagnostic> = items.into_iter().map(|(_, d)| d).collect();
    if omitted > 0 {
        kept.push(more(omitted));
    }
    *diags = kept;
}

/// 省いた件数の診断 (「ほか N 件」)。位置は無い。
pub(crate) fn more(count: usize) -> crate::Diagnostic {
    crate::Diagnostic {
        severity: crate::Severity::Warning,
        kind: crate::DiagKind::More { count },
        key: String::new(),
        value: String::new(),
        line: 0,
        column: 0,
        condition: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 先頭から数える素朴な方法。
    fn naive(text: &str, offset: usize) -> (u32, u32) {
        let head = &text[..offset];
        let line = head.matches('\n').count() + 1;
        let col = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (u32::try_from(line).unwrap(), u32::try_from(col).unwrap())
    }

    #[test]
    fn 行と桁は先頭から数えた値と同じ() {
        let text = "ab\nあい\n\nxyz\n";
        let idx = LineIndex::new(text);
        for (i, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            assert_eq!(idx.pos(i), naive(text, i), "{i}");
        }
        assert_eq!(idx.pos(10_000), naive(text, text.len()));
    }
}
