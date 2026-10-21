//! 端末を復元した後に stderr へ出す文言の定数表 (docs/04 §15.2 の `stderr.*` / `panic.*`)。言語ファイルには入れない
//! (パニックの経路で言語ファイルを引かないため)。**v0.1 は日本語の行だけ** (OQ-67)。パニック時の行の順は
//! `panic.header` → 退避の結果 (`stderr.recovery.*`。`saved` / `fallback` のときは `stderr.recovery.next` も) → `panic.crash_log` → `panic.report`。

use std::path::Path;

use eve_core::RescueOutcome;

pub const PANIC_HEADER: &str = "予期しないエラーのため evoeditor を終了しました。";
pub const RECOVERY_NEXT: &str = "次に evoeditor を起動したときに退避の場所を表示します。そのファイルを開いて内容を確かめてください。";

pub fn panic_crash_log(path: &Path) -> String {
    format!(
        "クラッシュログ: {} (文書の内容は含みません)",
        path.display()
    )
}

pub fn panic_report(url: &str) -> String {
    format!("問題を報告するときは、クラッシュログを Issue に添付してください: {url}")
}

/// 退避の結果の行 (`stderr.recovery.*`)。退避する文書が無かったときは空。
pub fn recovery_lines(o: &RescueOutcome) -> Vec<String> {
    if o.nothing_to_save() {
        return Vec::new();
    }
    let dir = o
        .dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let mut lines = Vec::new();
    match (&o.dir, o.written, o.complete) {
        (Some(_), w, true) if w == o.total && !o.fallback => {
            lines.push(format!(
                "保存されていない変更を退避しました: {dir} ({} 件)",
                o.total
            ));
            lines.push(RECOVERY_NEXT.to_owned());
        }
        (Some(_), w, true) if w == o.total => {
            lines.push(format!(
                "いつもの場所に書けなかったため、別の場所に退避しました: {dir} (早めに別の場所へ移してください)"
            ));
            lines.push(RECOVERY_NEXT.to_owned());
        }
        (Some(_), w, _) if w > 0 => {
            lines.push(format!(
                "{} 件のうち {w} 件だけを退避できました: {dir}",
                o.total
            ));
        }
        _ => {
            let reason = o.error.as_deref().unwrap_or("理由は不明です");
            lines.push(format!(
                "保存されていない変更を退避できませんでした: {reason}"
            ));
        }
    }
    if o.sensitive && o.dir.is_some() {
        lines.push(format!(
            "退避した中に秘密を含むかもしれないファイルがあります。要らなくなったら {dir} を削除してください。"
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn outcome(total: usize, written: usize, complete: bool, fallback: bool) -> RescueOutcome {
        RescueOutcome {
            dir: Some(PathBuf::from("C:\\s\\recovery\\20261021T000000Z-1")),
            total,
            written,
            complete,
            fallback,
            ..RescueOutcome::default()
        }
    }

    #[test]
    fn 退避した() {
        let l = recovery_lines(&outcome(2, 2, true, false));
        assert!(
            l[0].starts_with("保存されていない変更を退避しました:"),
            "{l:?}"
        );
        assert!(l[0].contains("(2 件)") && l[0].contains("20261021T000000Z-1"));
        assert_eq!(l[1], RECOVERY_NEXT);
    }

    #[test]
    fn 代替の退避先() {
        let l = recovery_lines(&outcome(1, 1, true, true));
        assert!(
            l[0].contains("別の場所に退避しました") && l[0].contains("早めに"),
            "{l:?}"
        );
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn 一部だけ() {
        let l = recovery_lines(&outcome(3, 1, false, false));
        assert!(l[0].contains("3 件のうち 1 件だけ"), "{l:?}");
        assert_eq!(l.len(), 1, "next は付けない");
    }

    #[test]
    fn 退避できなかった() {
        let mut o = outcome(1, 0, false, false);
        o.dir = None;
        o.error = Some("アクセスが拒否されました".into());
        let l = recovery_lines(&o);
        assert!(
            l[0].contains("退避できませんでした: アクセスが拒否されました"),
            "{l:?}"
        );
    }

    #[test]
    fn 文書が無ければ出さない_秘密は削除を促す() {
        assert!(recovery_lines(&RescueOutcome::default()).is_empty());
        let mut o = outcome(1, 1, true, false);
        o.sensitive = true;
        let l = recovery_lines(&o);
        assert!(
            l.iter().any(|x| x.contains("秘密を含むかもしれない")),
            "{l:?}"
        );
    }
}
