//! `cargo xtask bench --env ENV-W1 --out docs/perf/<版>/` の記録の書き出し (docs/09 §5.3、docs/11 §1 の `docs/perf/` の行)。
//!
//! ベンチ ID ごとの JSON (`<ベンチ ID>.json`: ベンチ ID・コミット・環境 ID・結果の行) と、表にした `summary.md`
//! (回数・p50 / p99 / p99.9 / 最大値・目標・判定) を書く。環境 ID だけを書き、基準機の固有情報は書かない (docs/11 §9.7)。
//!
//! **判定**: 目標があり、測った区間が目標の区間と同じものだけ「合格 / 不合格」にする。区間が違うもの (端末込みの近似・
//! 診断用) は「参考」とし、目標は備考に書く。判定の最終は人が `summary.md` を読んで決める (リリースのチェックリストの「性能」)。

use serde_json::Value;

/// 結果 1 行の見立て: (目標の文, 判定, 備考)。
pub(crate) struct Verdict {
    pub target: String,
    pub verdict: &'static str,
    pub note: &'static str,
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn judge(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "合格",
        Some(false) => "不合格",
        None => "参考",
    }
}

/// 目標 `p50 <= a` かつ `p99 <= b` (ミリ秒) の判定。
fn limits(v: &Value, p50: f64, p99: f64) -> Option<bool> {
    Some(f(v, "p50_ms")? <= p50 && f(v, "p99_ms")? <= p99)
}

pub(crate) fn verdict(v: &Value) -> Verdict {
    let id = v.get("id").and_then(Value::as_str).unwrap_or("");
    let t = |s: &str, ok: Option<bool>, note: &'static str| Verdict {
        target: s.to_owned(),
        verdict: judge(ok),
        note,
    };
    match id {
        // 計測用ビルド (bench-trace) のプロセス内の区間の記録。仕様 (01 §3.0.1) の区間そのもの
        "BM-101a" => t(
            "p50 8 ms / p99 15 ms",
            limits(v, 8.0, 15.0),
            "main 入口 → 最初の Frame の write",
        ),
        "BM-102a" => t(
            "p50 25 ms / p99 50 ms",
            limits(v, 25.0, 50.0),
            "main 入口 → 本文の最初の Frame の write (1 MB)",
        ),
        "BM-101b" => t(
            "p50 40 ms / p99 80 ms",
            limits(v, 40.0, 80.0),
            "spawn → 最初の Frame の write (壁時計)。ウイルス対策の条件を記録する",
        ),
        "BM-102b" => t(
            "p50 60 ms / p99 100 ms",
            limits(v, 60.0, 100.0),
            "spawn → 本文の最初の Frame の write (1 MB、壁時計)",
        ),
        "BM-103a" => t("(診断用)", None, "core 区間のみ。判定は BM-103b"),
        "BM-103b" => t(
            "p50 2 ms / p99 8 ms",
            limits(v, 2.0, 8.0),
            "入力の読取完了 → 描画の write (打鍵間隔 30 ms)",
        ),
        "BM-107" => t(
            "NFR-103 を維持 (p50 2 ms / p99 8 ms)",
            limits(v, 2.0, 8.0),
            "100 MB の保存中のカーソル移動 (入力の読取完了 → 描画の write)。保存時間は参考",
        ),
        // 端末込みの参考値 (ConPTY を挟む)
        "BM-101c" | "BM-102c" => t(
            "(参考)",
            None,
            "spawn → 最初の画面が端末に届くまで (ConPTY 経由の近似)",
        ),
        "BM-103c" | "BM-107c" => t(
            "(参考)",
            None,
            "端末込み (ConPTY の出力の周期 約 16 ms を含む)",
        ),
        "BM-106" => t("p99 100 ms", limits(v, f64::MAX, 100.0), ""),
        "BM-205" => t("(記録)", None, "配布ビルドの exe のサイズ"),
        _ if id.starts_with("BM-104") => {
            // 全読込完了の p50 の目標 (NFR-104)。Shift_JIS は 1.5 倍まで
            let (target, limit_ms) = match id {
                "BM-104-10m" => ("p50 150 ms", Some(150.0)),
                "BM-104-100m" => ("p50 1000 ms", Some(1000.0)),
                "BM-104-100m-sjis" => ("p50 1500 ms (UTF-8 の 1.5 倍)", Some(1500.0)),
                _ => ("(記録)", None),
            };
            let time_ok = limit_ms.and_then(|l| f(v, "p50_ms").map(|p| p <= l));
            let mem_ok = f(v, "mem_delta_bytes")
                .zip(f(v, "mem_limit_bytes"))
                .map(|(d, l)| d <= l);
            let ok = match (time_ok, mem_ok) {
                (Some(a), Some(b)) => Some(a && b),
                (a, b) => a.or(b),
            };
            Verdict {
                target: format!("{target}、メモリ: デコード後 × 1.5 + 15 MB"),
                verdict: judge(ok),
                note: "全読込完了 (進捗表示の区間は別)",
            }
        }
        _ => Verdict {
            target: String::new(),
            verdict: "参考",
            note: "",
        },
    }
}

fn fmt_ms(v: &Value, k: &str) -> String {
    f(v, k).map_or("—".to_owned(), |x| format!("{x:.2}"))
}

/// 1 行の見出し (ID + ケース + 操作)。
fn label(v: &Value) -> String {
    let mut s = v
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned();
    for k in ["case", "op"] {
        if let Some(x) = v.get(k).and_then(Value::as_str) {
            s.push(' ');
            s.push_str(x);
        }
    }
    s
}

/// `dir` に `<ベンチ ID>.json` と `summary.md` を書く。
pub(crate) fn write(
    dir: &std::path::Path,
    env: &str,
    commit: &str,
    quick: bool,
    lines: &[String],
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut rows: Vec<Value> = Vec::new();
    for l in lines {
        rows.push(serde_json::from_str(l).map_err(|e| format!("結果の行を読めません: {e}: {l}"))?);
    }
    // ベンチ ID ごとのファイル
    let mut ids: Vec<String> = rows
        .iter()
        .filter_map(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    ids.sort();
    ids.dedup();
    for id in &ids {
        let results: Vec<&Value> = rows
            .iter()
            .filter(|v| v.get("id").and_then(Value::as_str) == Some(id))
            .collect();
        let doc = serde_json::json!({
            "id": id,
            "commit": commit,
            "env": env,
            "quick": quick,
            "results": results,
        });
        let path = dir.join(format!("{id}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n",
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
    }

    let mut md = String::new();
    md.push_str(&format!("# 性能の記録 — {env}\n\n"));
    md.push_str(&format!(
        "- コミット: `{commit}`\n- 環境 ID: `{env}` (構成は `docs/perf/environments.md`)\n"
    ));
    if quick {
        md.push_str("- **`--quick` の結果です。動作確認用で、記録には使えません**\n");
    }
    md.push_str(
        "- 判定は、測った区間が目標の区間と同じものだけ自動で付ける。「参考」は区間が違う・診断用のもの (備考に理由)。\
         リリースのチェックリストの「性能」の最終判定は、この表を読んで人が行う。\n\n",
    );
    md.push_str("| ベンチ | 回数 | p50 (ms) | p99 (ms) | p99.9 (ms) | 最大 (ms) | 目標 | 判定 | 備考 |\n|---|---|---|---|---|---|---|---|---|\n");
    for v in &rows {
        let ver = verdict(v);
        let n = v
            .get("n")
            .or_else(|| v.get("runs"))
            .and_then(Value::as_u64)
            .map_or("—".to_owned(), |n| n.to_string());
        let (p50, p99, p999, max) = if f(v, "p50_us").is_some() {
            // マイクロ秒の行 (BM-103a) はミリ秒にそろえる
            let ms = |k: &str| f(v, k).map_or("—".to_owned(), |x| format!("{:.3}", x / 1000.0));
            (ms("p50_us"), ms("p99_us"), ms("p999_us"), ms("max_us"))
        } else {
            (
                fmt_ms(v, "p50_ms"),
                fmt_ms(v, "p99_ms"),
                fmt_ms(v, "p999_ms"),
                fmt_ms(v, "max_ms"),
            )
        };
        let note = if v.get("bytes").is_some() {
            format!("{} バイト。{}", v["bytes"], ver.note)
        } else {
            ver.note.to_owned()
        };
        md.push_str(&format!(
            "| {} | {n} | {p50} | {p99} | {p999} | {max} | {} | {} | {note} |\n",
            label(v),
            ver.target,
            ver.verdict
        ));
    }
    if !rows
        .iter()
        .any(|v| v.get("id").and_then(Value::as_str) == Some("BM-101a"))
    {
        md.push_str("\n**BM-101a / 102a / 101b / 102b / 103b の判定区間が無い**: 計測用ビルド (`bench-trace`) で測っていない。\n");
    }
    let path = dir.join("summary.md");
    std::fs::write(&path, md).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn 区間が同じものだけ合否を付ける() {
        let ok = verdict(&v(r#"{"id":"BM-101b","p50_ms":38.0,"p99_ms":53.0}"#));
        assert_eq!(ok.verdict, "合格");
        let ng = verdict(&v(r#"{"id":"BM-101b","p50_ms":38.0,"p99_ms":90.0}"#));
        assert_eq!(ng.verdict, "不合格");
        let r = verdict(&v(r#"{"id":"BM-103c","p50_ms":15.0,"p99_ms":16.0}"#));
        assert_eq!(r.verdict, "参考", "端末込みは参考");
        let j = verdict(&v(r#"{"id":"BM-103b","p50_ms":1.5,"p99_ms":3.0}"#));
        assert_eq!(j.verdict, "合格", "判定区間は合否を付ける");
        let d = verdict(&v(r#"{"id":"BM-103a","p50_us":1000.0}"#));
        assert_eq!(d.verdict, "参考");
    }

    #[test]
    fn 読込は時間とメモリの両方で判定する() {
        let ok = verdict(&v(
            r#"{"id":"BM-104-100m","p50_ms":900.0,"mem_delta_bytes":100,"mem_limit_bytes":200}"#,
        ));
        assert_eq!(ok.verdict, "合格");
        let mem = verdict(&v(
            r#"{"id":"BM-104-100m","p50_ms":900.0,"mem_delta_bytes":300,"mem_limit_bytes":200}"#,
        ));
        assert_eq!(mem.verdict, "不合格");
        let slow = verdict(&v(
            r#"{"id":"BM-104-100m-sjis","p50_ms":1600.0,"mem_delta_bytes":1,"mem_limit_bytes":2}"#,
        ));
        assert_eq!(slow.verdict, "不合格");
    }

    #[test]
    fn 長大行は_p99_100ms() {
        assert_eq!(
            verdict(&v(
                r#"{"id":"BM-106","op":"end","p50_ms":1.0,"p99_ms":99.0}"#
            ))
            .verdict,
            "合格"
        );
        assert_eq!(
            verdict(&v(
                r#"{"id":"BM-106","op":"end","p50_ms":1.0,"p99_ms":120.0}"#
            ))
            .verdict,
            "不合格"
        );
    }
}
