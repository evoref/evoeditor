//! シナリオ DSL (TOML) のランナー (docs/11 §6.3、NFR-707、v0.2)。`Harness` の薄い写像で、DSL にできることは Harness にもできる。
//!
//! M10 で入れた範囲: `[meta]`・`[setup]` (`size`・`caps = "tui_default"`・`config`・`fs` の `text`)・`[[step]]` (`open`・`run_jobs`・
//! `keys`・`text`・`paste`・`advance_ms`・`resize`・`cmd` + `args`)・`[[assert]]` (`at`・`fs`・`query` + `expect`・`frame_contains`・
//! `events`・`overlay`・`no_panic`)。まだのもの (Frame のゴールデン `frame`、`effects`、`status`、`fs` の `bytes` / `file`、
//! `locale`・`caps` の他のプリセット) は読み込むと「未対応」で失敗する。`now`・`seed` は受け付けるが、今のハーネスは時刻 0 から始まり
//! 乱数を使わないので効果がない。
//!
//! パスはワークスペース相対で、偽 FS の `/w` の下に置く (`"a.txt"` は `/w/a.txt`)。

use std::collections::BTreeMap;
use std::path::Path;

use eve_core::{ArgValue, Args};
use eve_view::{CommandId, EditorEvent};
use serde::Deserialize;

use crate::Harness;

/// 1 本のシナリオ。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub meta: Meta,
    #[serde(default)]
    pub setup: Setup,
    #[serde(default)]
    pub step: Vec<Step>,
    #[serde(default, rename = "assert")]
    pub asserts: Vec<Assert>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub verifies: Vec<String>,
    pub since: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    pub size: Option<[u16; 2]>,
    pub caps: Option<String>,
    pub now: Option<String>,
    pub seed: Option<u64>,
    pub config: Option<String>,
    pub locale: Option<String>,
    #[serde(default)]
    pub fs: BTreeMap<String, FsEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsEntry {
    pub text: Option<String>,
    pub bytes: Option<String>,
    pub file: Option<String>,
}

/// 1 ステップ。1 種類の入力だけを書く (`args` は `cmd` に添える)。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub open: Option<String>,
    pub run_jobs: Option<bool>,
    pub keys: Option<String>,
    pub text: Option<String>,
    pub paste: Option<String>,
    pub advance_ms: Option<u64>,
    pub resize: Option<[u16; 2]>,
    pub cmd: Option<String>,
    pub args: Option<toml::Table>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub kind: String,
    pub path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assert {
    pub at: Option<usize>,
    pub fs: Option<BTreeMap<String, FsEntry>>,
    pub query: Option<QuerySpec>,
    pub expect: Option<toml::Table>,
    pub frame: Option<String>,
    pub frame_contains: Option<Vec<String>>,
    pub events: Option<Vec<String>>,
    pub effects: Option<toml::Value>,
    pub status: Option<toml::Table>,
    pub overlay: Option<String>,
    pub no_panic: Option<bool>,
}

fn ws_path(p: &str) -> String {
    format!("/w/{}", p.trim_start_matches('/'))
}

/// ファイルを読んで型と規則を検証する。`rel` は `tests/scenarios/` からの相対パス (`file/save_crlf.toml`)。
pub fn load(text: &str, rel: &str) -> Result<Scenario, Vec<String>> {
    let sc: Scenario = toml::from_str(text).map_err(|e| vec![format!("{rel}: {e}")])?;
    let errs = validate(&sc, rel);
    if errs.is_empty() { Ok(sc) } else { Err(errs) }
}

/// 型で表せない規則 (11 §6.3 の表)。存在しない CommandId もここで見る。
pub fn validate(sc: &Scenario, rel: &str) -> Vec<String> {
    let mut e = Vec::new();
    let want_id = rel.trim_end_matches(".toml").replace('\\', "/");
    if sc.meta.id != want_id {
        e.push(format!(
            "{rel}: [meta] id `{}` はパス `{want_id}` と一致しません",
            sc.meta.id
        ));
    }
    if !sc.meta.since.starts_with('v') {
        e.push(format!(
            "{rel}: [meta] since `{}` は版名ではありません",
            sc.meta.since
        ));
    }
    if let Some(c) = &sc.setup.caps
        && c != "tui_default"
    {
        e.push(format!(
            "{rel}: [setup] caps `{c}` はまだ未対応です (tui_default のみ)"
        ));
    }
    if sc.setup.locale.is_some() {
        e.push(format!("{rel}: [setup] locale はまだ未対応です"));
    }
    let fs_entries = sc
        .setup
        .fs
        .iter()
        .chain(sc.asserts.iter().flat_map(|a| a.fs.iter().flatten()));
    for (p, f) in fs_entries {
        if f.bytes.is_some() || f.file.is_some() || f.text.is_none() {
            e.push(format!(
                "{rel}: fs `{p}` は text だけに対応しています (bytes / file は未対応)"
            ));
        }
    }
    let registry = Harness::new()
        .editor
        .registry()
        .iter()
        .map(|s| s.id.clone())
        .collect::<Vec<CommandId>>();
    for (i, s) in sc.step.iter().enumerate() {
        let n = [
            s.open.is_some(),
            s.run_jobs.is_some(),
            s.keys.is_some(),
            s.text.is_some(),
            s.paste.is_some(),
            s.advance_ms.is_some(),
            s.resize.is_some(),
            s.cmd.is_some(),
        ]
        .into_iter()
        .filter(|b| *b)
        .count();
        if n != 1 {
            e.push(format!(
                "{rel}: step {} は入力を 1 種類だけ書きます ({n} 種類)",
                i + 1
            ));
        }
        if s.args.is_some() && s.cmd.is_none() {
            e.push(format!("{rel}: step {} の args は cmd に添えます", i + 1));
        }
        if let Some(c) = &s.cmd
            && !registry.contains(&CommandId::new(c))
        {
            e.push(format!(
                "{rel}: step {} の CommandId `{c}` は存在しません",
                i + 1
            ));
        }
    }
    for (i, a) in sc.asserts.iter().enumerate() {
        let k = i + 1;
        if a.frame.is_some() || a.effects.is_some() || a.status.is_some() {
            e.push(format!(
                "{rel}: assert {k} の frame / effects / status はまだ未対応です"
            ));
        }
        if a.query.is_some() != a.expect.is_some() {
            e.push(format!(
                "{rel}: assert {k} の query と expect は組で書きます"
            ));
        }
        if let Some(at) = a.at
            && (at == 0 || at > sc.step.len())
        {
            e.push(format!(
                "{rel}: assert {k} の at = {at} は step の範囲外です"
            ));
        }
    }
    e
}

fn args_of(t: &toml::Table) -> Result<Args, String> {
    let mut a = Args::new();
    for (k, v) in t {
        let v = match v {
            toml::Value::Integer(n) => ArgValue::Int(*n),
            toml::Value::String(s) => ArgValue::Str(s.clone()),
            toml::Value::Boolean(b) => ArgValue::Bool(*b),
            other => return Err(format!("args.{k} の型 ({other}) には対応していません")),
        };
        a = a.with(k, v);
    }
    Ok(a)
}

fn event_name(ev: &EditorEvent) -> String {
    let d = format!("{ev:?}");
    d.split([' ', '(', '{']).next().unwrap_or("").to_owned()
}

fn check_expect(h: &Harness, q: &QuerySpec, expect: &toml::Table) -> Result<(), String> {
    let st = h.editor.state();
    let mut errs = Vec::new();
    let mut want = |key: &str, got: String, exp: &toml::Value| {
        let exp_s = match exp {
            toml::Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        if got != exp_s {
            errs.push(format!("{}.{key}: 期待 {exp_s:?}、実際 {got:?}", q.kind));
        }
    };
    match q.kind.as_str() {
        "documents" => {
            for (k, v) in expect {
                match k.as_str() {
                    "count" => want(k, st.documents_len().to_string(), v),
                    _ => return Err(format!("documents の expect に `{k}` はありません")),
                }
            }
        }
        "document" | "text" => {
            let id = match &q.path {
                Some(p) => {
                    let p = ws_path(p);
                    let Ok(eve_view::Answer::Documents(docs)) =
                        h.query(&eve_view::Query::Documents)
                    else {
                        return Err("文書の一覧を得られません".to_owned());
                    };
                    docs.into_iter()
                        .find(|d| {
                            st.document(d.doc)
                                .and_then(|x| x.path.as_ref())
                                .is_some_and(|x| x.display() == p)
                        })
                        .map(|d| d.doc)
                        .ok_or_else(|| format!("{p} の文書が開いていません"))?
                }
                None => h.doc_id(),
            };
            let d = st.document(id).ok_or("文書がありません")?;
            for (k, v) in expect {
                match k.as_str() {
                    "dirty" => want(k, d.is_dirty().to_string(), v),
                    "text" => want(k, d.to_string_lossy(), v),
                    "read_only" => want(k, (!d.is_editable()).to_string(), v),
                    "encoding" => want(k, d.format.encoding.name().to_lowercase(), v),
                    "eol" => {
                        let s = d.format.eol_stats;
                        let got = if s.is_mixed() {
                            "mixed"
                        } else if s.crlf > 0 {
                            "crlf"
                        } else if s.cr > 0 {
                            "cr"
                        } else {
                            "lf"
                        };
                        want(k, got.to_owned(), v);
                    }
                    _ => return Err(format!("{} の expect に `{k}` はありません", q.kind)),
                }
            }
        }
        "selection" => {
            let w = st.first_window().ok_or("Window がありません")?;
            let v = st
                .view(st.window_view(w).ok_or("View がありません")?)
                .ok_or("View がありません")?;
            let p = *v.selection.primary();
            for (k, e) in expect {
                match k.as_str() {
                    "empty" => want(k, v.selection.iter().all(|r| r.is_empty()).to_string(), e),
                    "count" => want(k, v.selection.ranges().len().to_string(), e),
                    "anchor" => want(k, p.anchor.0.to_string(), e),
                    "head" => want(k, p.head.0.to_string(), e),
                    _ => return Err(format!("selection の expect に `{k}` はありません")),
                }
            }
        }
        other => {
            return Err(format!(
                "query.kind `{other}` はまだ未対応です (documents / document / text / selection)"
            ));
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("\n"))
    }
}

fn check_assert(h: &mut Harness, a: &Assert, k: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut fail = |m: String| out.push(format!("assert {k}: {m}"));
    if let Some(fs) = &a.fs {
        for (p, f) in fs {
            let got = h.fs.read(&ws_path(p));
            let want = f.text.as_deref().map(str::as_bytes);
            if got.as_deref() != want {
                fail(format!(
                    "fs `{p}`: 期待 {:?}、実際 {:?}",
                    f.text,
                    got.map(|b| String::from_utf8_lossy(&b).into_owned())
                ));
            }
        }
    }
    if let (Some(q), Some(e)) = (&a.query, &a.expect)
        && let Err(m) = check_expect(h, q, e)
    {
        fail(m);
    }
    if let Some(needles) = &a.frame_contains {
        let text = h.frame_text();
        for n in needles {
            if !text.contains(n.as_str()) {
                fail(format!("Frame に {n:?} がありません\n{text}"));
            }
        }
    }
    if let Some(want) = &a.events {
        let got: Vec<String> = h.events().iter().map(event_name).collect();
        let mut it = got.iter();
        if !want.iter().all(|w| it.any(|g| g == w)) {
            fail(format!(
                "events: 部分列 {want:?} がありません (実際 {got:?})"
            ));
        }
    }
    if let Some(o) = &a.overlay
        && h.overlay() != o
    {
        fail(format!("overlay: 期待 {o:?}、実際 {:?}", h.overlay()));
    }
    out
}

/// 実行の結果 (最後の Frame の ASCII。`--print-frame` で出す)。
pub struct Outcome {
    pub frame: String,
}

/// シナリオを実行し、失敗した assert を返す。パニックは `no_panic` の失敗として扱う。
pub fn run(sc: &Scenario) -> Result<Outcome, Vec<String>> {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_inner(sc)));
    match res {
        Ok(r) => r,
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            if sc.asserts.iter().any(|a| a.no_panic == Some(false)) {
                Ok(Outcome {
                    frame: String::new(),
                })
            } else {
                Err(vec![format!("パニックしました: {msg}")])
            }
        }
    }
}

fn run_inner(sc: &Scenario) -> Result<Outcome, Vec<String>> {
    let mut h = Harness::new();
    if let Some([c, r]) = sc.setup.size {
        h = h.with_size(c, r);
    }
    if let Some(text) = &sc.setup.config {
        let loaded = eve_config::load_str(text);
        if !loaded.diagnostics.is_empty() {
            return Err(vec![format!(
                "[setup] config に診断があります: {:?}",
                loaded.diagnostics
            )]);
        }
        h.editor.state_mut().set_config(loaded.config);
    }
    for (p, f) in &sc.setup.fs {
        h = h.with_file(&ws_path(p), f.text.as_deref().unwrap_or("").as_bytes());
    }
    let mut errs = Vec::new();
    let run_asserts = |h: &mut Harness, at: Option<usize>, errs: &mut Vec<String>| {
        for (i, a) in sc.asserts.iter().enumerate() {
            if a.at == at {
                errs.extend(check_assert(h, a, i + 1));
            }
        }
    };
    for (i, s) in sc.step.iter().enumerate() {
        if let Some(p) = &s.open {
            h.open(&ws_path(p));
        } else if s.run_jobs == Some(true) {
            h.run_jobs();
        } else if let Some(k) = &s.keys {
            h.keys(k);
        } else if let Some(t) = &s.text {
            h.text(t);
        } else if let Some(t) = &s.paste {
            h.paste(t);
        } else if let Some(ms) = s.advance_ms {
            h.advance_ms(ms);
        } else if let Some([c, r]) = s.resize {
            h.resize(c, r);
        } else if let Some(c) = &s.cmd {
            let args = match s.args.as_ref().map(args_of).transpose() {
                Ok(a) => a.unwrap_or_default(),
                Err(m) => {
                    errs.push(format!("step {}: {m}", i + 1));
                    continue;
                }
            };
            h.cmd_with(c, args);
        }
        run_asserts(&mut h, Some(i + 1), &mut errs);
    }
    run_asserts(&mut h, None, &mut errs);
    let frame = h.frame_text();
    if errs.is_empty() {
        Ok(Outcome { frame })
    } else {
        Err(errs)
    }
}

/// `dir` (`tests/scenarios`) の下の `.toml` を相対パスの昇順で集める。
pub fn collect(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if p.extension().is_some_and(|x| x == "toml") {
                let rel = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, p));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}
