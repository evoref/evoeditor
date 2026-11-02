//! `cargo xtask accept-linux`: Linux の受入の手順 (docs/compat/checklist.md の A〜G と L) のうち、端末の中の動作を
//! **tmux を擬似端末にして** 自動で確かめる (docs/09 §3.7、Linux)。tmux の `send-keys` / `paste-buffer -p` でキーと
//! 貼り付けを送り、`capture-pane` で画面 (`-e` で色と属性) を読む。WSL では Windows のクリップボードとの往復も
//! `powershell.exe` で確かめる (L4)。
//!
//! 自動にできないもの (日本語入力の候補窓、Windows Terminal が送るキー・タブを閉じたときの SIGHUP・横取りするキー、
//! 右クリックの貼り付け、色の見た目の最終判断) は手で確かめる (checklist の L と `_template/linux-wsl.md`)。
//!
//! ```text
//! cargo xtask accept-linux [--archive <evoeditor-<版>-<三つ組>.tar.gz>] [--only A1,C2,...] [--keep]
//! ```
//!
//! `--archive` を省くと `cargo xtask dist --target <この機械の musl>` で作る。アーカイブの隣の `SHA256SUMS` で
//! ハッシュを検証し (`SHA256SUMS.minisig` と `minisign` があれば署名も)、README の Linux の手順どおりに作業用の HOME へ
//! 入れて (L1)、以降はその `~/.local/bin/evoeditor` を使う。各項目は README の推奨設定の tmux の中で動かし (L7)、
//! L8 だけ推奨設定なしで動かす。結果は `target/accept-linux/result.md` に書く (`linux-wsl.md` の記録の元)。
//! L4 は Windows のクリップボードを書き換える (終わったら元の内容に戻す)。

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::util::{run, workspace_root};

/// tmux のサーバのソケット名 (利用者の tmux と混ざらないように分ける)。
const SOCK: &str = "eve-accept";

/// README の「既知の制約 (Linux)」の tmux の推奨設定 (L7)。`remain-on-exit` は終了コードを読むため。
const RECOMMENDED: &str = "set -s escape-time 10\nset -g set-clipboard on\nset -s extended-keys on\nset -g focus-events on\nset -g remain-on-exit on\n";
/// 推奨設定なし (L8。`tmux -f /dev/null` 相当)。
const BARE: &str = "set -g remain-on-exit on\n";

/// 編集の画面が出たことの目印 (ヒントバー)。
const HINT: &str = "^Q 終了";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Recommended,
    Bare,
}

/// 1 項目の結果。`Err` は不合格 (理由)。
enum Outcome {
    Pass(String),
    Skip(String),
}

type CheckResult = Result<Outcome, String>;

/// 起動に加える環境変数。
type Envs<'a> = &'a [(&'a str, &'a str)];

fn pass(note: impl Into<String>) -> CheckResult {
    Ok(Outcome::Pass(note.into()))
}

struct Check {
    id: &'static str,
    item: &'static str,
    run: fn(&Ctx) -> CheckResult,
}

const CHECKS: &[Check] = &[
    Check {
        id: "A1",
        item: "A1・A2 起動と終了 (端末が元に戻る)",
        run: a1_start_quit,
    },
    Check {
        id: "A3",
        item: "A3 --version と --help",
        run: a3_version_help,
    },
    Check {
        id: "A4",
        item: "A4 存在しないパスを新規として保存",
        run: a4_new_file,
    },
    Check {
        id: "A5",
        item: "A5 日本語のファイル名",
        run: a5_japanese_name,
    },
    Check {
        id: "C1",
        item: "C1 リサイズ (大・小・極小)",
        run: c1_resize,
    },
    Check {
        id: "C2",
        item: "C2 Ctrl+Z と Ctrl+Shift+Z の区別 (tmux の中)",
        run: c2_ctrl_shift,
    },
    Check {
        id: "C5",
        item: "C5 色の段階 (truecolor / 256 / 16 / none / NO_COLOR)",
        run: c5_color_modes,
    },
    Check {
        id: "D1",
        item: "D1・L5 ブラケットペーストの 100 行と 1 回の元に戻す",
        run: d1_paste_lines,
    },
    Check {
        id: "D3",
        item: "D3・L5 貼り付けの制御文字 (paste-injection.txt)",
        run: d3_paste_injection,
    },
    Check {
        id: "D4",
        item: "D4・L4 Windows のクリップボードとの往復 (WSL)",
        run: d4_clipboard,
    },
    Check {
        id: "D6",
        item: "D6 数 MB の貼り付け",
        run: d6_big_paste,
    },
    Check {
        id: "E1",
        item: "E1・E3 検索 (次・前・折り返し・大小・日本語)",
        run: e1_search,
    },
    Check {
        id: "E2",
        item: "E2 すべて置換と 1 回の元に戻す",
        run: e2_replace_all,
    },
    Check {
        id: "F1",
        item: "F1 UTF-8 (CRLF) の保存",
        run: f1_crlf,
    },
    Check {
        id: "F2",
        item: "F2 Shift_JIS の表示と保存",
        run: f2_shift_jis,
    },
    Check {
        id: "F3",
        item: "F3 読み取り専用のファイル",
        run: f3_read_only,
    },
    Check {
        id: "F4",
        item: "F4 外部で変更されたファイルの保存",
        run: f4_external_change,
    },
    Check {
        id: "F5",
        item: "F5 未保存で終了の確認とキャンセル",
        run: f5_quit_cancel,
    },
    Check {
        id: "F6",
        item: "F6 名前を付けて保存の上書きの確認",
        run: f6_overwrite,
    },
    Check {
        id: "G1",
        item: "G1 100 MB の進捗と取り消し",
        run: g1_big_cancel,
    },
    Check {
        id: "G2",
        item: "G2 512 MiB 超の警告",
        run: g2_huge_warning,
    },
    Check {
        id: "G3",
        item: "G3 100 MB の保存中の入力",
        run: g3_big_save,
    },
    Check {
        id: "L2",
        item: "L2・L7 SIGHUP (kill-session) の退避と起動時の案内",
        run: l2_sighup,
    },
    Check {
        id: "L3",
        item: "L3 Ctrl+K Z のサスペンドと fg (大きさの変更)",
        run: l3_suspend,
    },
    Check {
        id: "L8",
        item: "L8 tmux の推奨設定なしの Esc と Ctrl+Shift",
        run: l8_bare,
    },
];

/// 受入の共通の材料。
struct Ctx {
    /// 作業用の HOME に入れた `~/.local/bin/evoeditor`
    exe: PathBuf,
    home: PathBuf,
    /// 作業のファイルを置くディレクトリ
    work: PathBuf,
    recommended: PathBuf,
    bare: PathBuf,
    version: String,
}

impl Ctx {
    fn file(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let p = self.work.join(name);
        fs::write(&p, bytes).map_err(|e| format!("{} を書けません: {e}", p.display()))?;
        Ok(p)
    }

    /// 存在しないことを確かめた作業のパス (新規のファイルの保存先)。
    fn fresh(&self, name: &str) -> PathBuf {
        let p = self.work.join(name);
        let _ = fs::remove_file(&p);
        p
    }

    fn state(&self, label: &str) -> PathBuf {
        self.work.join("state").join(label)
    }
}

/// tmux の中の 1 つの端末 (セッション)。落とすとサーバごと終わらせる。
struct Pane {
    conf: PathBuf,
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

impl Pane {
    /// evoeditor を `args` で起動する (状態のディレクトリは `state` に分ける)。
    fn start(
        ctx: &Ctx,
        profile: Profile,
        state: &str,
        (cols, rows): (u16, u16),
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Result<Pane, String> {
        let mut argv = env_prefix(ctx, state, envs);
        argv.push(ctx.exe.display().to_string());
        argv.extend(args.iter().map(|a| (*a).to_owned()));
        Pane::start_argv(ctx, profile, (cols, rows), &argv)
    }

    fn start_argv(
        ctx: &Ctx,
        profile: Profile,
        (cols, rows): (u16, u16),
        argv: &[String],
    ) -> Result<Pane, String> {
        let conf = match profile {
            Profile::Recommended => ctx.recommended.clone(),
            Profile::Bare => ctx.bare.clone(),
        };
        let pane = Pane { conf };
        let _ = pane.tmux(&["kill-server"]);
        let (c, r) = (cols.to_string(), rows.to_string());
        let work = ctx.work.display().to_string();
        let mut args = vec!["new-session", "-d", "-x", &c, "-y", &r, "-c", &work];
        args.extend(argv.iter().map(String::as_str));
        pane.tmux(&args)?;
        Ok(pane)
    }

    fn tmux(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new("tmux")
            .arg("-L")
            .arg(SOCK)
            .arg("-f")
            .arg(&self.conf)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("tmux を実行できません: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "tmux {}: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn keys(&self, keys: &[&str]) -> Result<(), String> {
        let mut args = vec!["send-keys"];
        args.extend_from_slice(keys);
        self.tmux(&args).map(drop)
    }

    /// 文字列をそのまま打つ (キーの名前として解釈させない)。
    fn text(&self, s: &str) -> Result<(), String> {
        self.tmux(&["send-keys", "-l", s]).map(drop)
    }

    /// 端末の貼り付け (アプリがブラケットペーストを有効にしていれば `ESC[200~` … `ESC[201~` で囲む)。
    fn paste(&self, ctx: &Ctx, bytes: &[u8]) -> Result<(), String> {
        let f = ctx.file("paste.bin", bytes)?;
        let f = f.display().to_string();
        self.tmux(&["load-buffer", "-b", "eve", &f])?;
        self.tmux(&["paste-buffer", "-p", "-d", "-b", "eve"])
            .map(drop)
    }

    /// Esc を送って少し待つ (直後のキーと続けて届くと Alt+キーとみなされる)。
    fn esc(&self) -> Result<(), String> {
        self.keys(&["Escape"])?;
        pause(150);
        Ok(())
    }

    fn screen(&self) -> Result<String, String> {
        self.tmux(&["capture-pane", "-p"])
    }

    fn screen_styled(&self) -> Result<String, String> {
        self.tmux(&["capture-pane", "-p", "-e"])
    }

    fn format(&self, f: &str) -> Result<String, String> {
        Ok(self.tmux(&["display-message", "-p", f])?.trim().to_owned())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let (c, r) = (cols.to_string(), rows.to_string());
        self.tmux(&["resize-window", "-x", &c, "-y", &r]).map(drop)
    }

    fn alive(&self) -> bool {
        self.format("#{pane_dead}").is_ok_and(|d| d == "0")
    }

    /// 画面が `pred` を満たすまで待つ。
    fn wait(&self, what: &str, ms: u64, pred: impl Fn(&str) -> bool) -> Result<String, String> {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            let s = self.screen()?;
            if pred(&s) {
                return Ok(s);
            }
            if Instant::now() >= end {
                return Err(format!(
                    "{what}: {:.1} 秒待っても出ません。画面:\n{}",
                    ms as f32 / 1000.0,
                    indent(&s)
                ));
            }
            pause(100);
        }
    }

    fn ready(&self) -> Result<String, String> {
        self.wait("編集の画面", 10_000, |s| s.contains(HINT))
    }

    /// 終わるまで待ち、終了コードを返す (シグナルで終わったら `sig<番号>`)。
    fn wait_exit(&self, ms: u64) -> Result<String, String> {
        let end = Instant::now() + Duration::from_millis(ms);
        while self.format("#{pane_dead}")? != "1" {
            if Instant::now() >= end {
                return Err(format!(
                    "{:.1} 秒待っても終わりません。画面:\n{}",
                    ms as f32 / 1000.0,
                    indent(&self.screen()?)
                ));
            }
            pause(100);
        }
        let sig = self.format("#{pane_dead_signal}")?;
        if !sig.is_empty() && sig != "0" {
            return Ok(format!("sig{sig}"));
        }
        self.format("#{pane_dead_status}")
    }

    /// Ctrl+Q で終わり、終了コード 0 を確かめる。
    fn quit(&self) -> Result<(), String> {
        self.keys(&["C-q"])?;
        let code = self.wait_exit(5_000)?;
        if code == "0" {
            Ok(())
        } else {
            Err(format!("Ctrl+Q の終了コードが 0 ではありません: {code}"))
        }
    }
}

fn env_prefix(ctx: &Ctx, state: &str, envs: &[(&str, &str)]) -> Vec<String> {
    let mut v = vec![
        "env".to_owned(),
        format!("HOME={}", ctx.home.display()),
        "EVE_LANG=ja".to_owned(),
        format!("EVE_STATE_DIR={}", ctx.state(state).display()),
    ];
    v.extend(envs.iter().map(|(k, val)| format!("{k}={val}")));
    v
}

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn indent(s: &str) -> String {
    s.trim_end()
        .lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ファイルが `pred` を満たすまで待つ。
fn wait_file(path: &Path, ms: u64, pred: impl Fn(&[u8]) -> bool) -> Result<Vec<u8>, String> {
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        let got = fs::read(path).ok();
        if let Some(b) = &got
            && pred(b)
        {
            return Ok(got.unwrap_or_default());
        }
        if Instant::now() >= end {
            return Err(match got {
                None => format!("{} ができません", file_name(path)),
                Some(b) => format!(
                    "{} の内容が期待と違います ({} バイト): {:?}",
                    file_name(path),
                    b.len(),
                    String::from_utf8_lossy(&b[..b.len().min(200)])
                ),
            });
        }
        pause(100);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// 画面の第 `n` 行 (0 始まり)。
fn line(screen: &str, n: usize) -> &str {
    screen.lines().nth(n).unwrap_or_default()
}

/// ステータスバーの行 (`行 <n>, 桁 <m>` を含む行)。
fn status_line(screen: &str) -> Option<&str> {
    screen
        .lines()
        .find(|l| l.contains("行 ") && l.contains(", 桁 "))
}

/// 表示幅 (東アジアの全角と絵文字を 2 とする近似。ステータスバーの幅の確認に使う)。
fn width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let u = c as u32;
            let wide = (0x1100..=0x115F).contains(&u)
                || (0x2E80..=0xA4CF).contains(&u)
                || (0xAC00..=0xD7A3).contains(&u)
                || (0xF900..=0xFAFF).contains(&u)
                || (0xFE30..=0xFE4F).contains(&u)
                || (0xFF00..=0xFF60).contains(&u)
                || (0xFFE0..=0xFFE6).contains(&u)
                || (0x1F300..=0x1FAFF).contains(&u);
            if wide { 2 } else { 1 }
        })
        .sum()
}

// ---------------------------------------------------------------------------------------------
// 入口

pub fn run_cmd(args: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("accept-linux は Linux (WSL を含む) だけで動きます".into());
    }
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let only: Vec<String> = value("--only")
        .map(|v| v.split(',').map(|s| s.trim().to_uppercase()).collect())
        .unwrap_or_default();
    let keep = args.iter().any(|a| a == "--keep");
    for tool in ["tmux", "sha256sum", "tar", "iconv"] {
        if Command::new("sh")
            .args(["-c", &format!("command -v {tool}")])
            .stdout(Stdio::null())
            .status()
            .map_or(true, |s| !s.success())
        {
            return Err(format!("{tool} が見つかりません (受入の自動化に要ります)"));
        }
    }

    let root = workspace_root();
    let archive = match value("--archive") {
        Some(a) => PathBuf::from(a),
        None => {
            let triple = format!("{}-unknown-linux-musl", std::env::consts::ARCH);
            run(
                "配布物の作成 (cargo xtask dist)",
                Command::new("cargo").args(["xtask", "dist", "--target", &triple]),
            )?;
            find_archive(&root.join("target/dist"), &triple)?
        }
    };
    let archive = archive
        .canonicalize()
        .map_err(|e| format!("{} がありません: {e}", archive.display()))?;

    let work_root = root.join("target/accept-linux");
    // F3 で読み取り専用にしたファイルがあっても消せる (消すのはディレクトリの書込権限)
    let _ = fs::remove_dir_all(&work_root);
    let home = work_root.join("home");
    let work = work_root.join("work");
    for d in [&home, &work] {
        fs::create_dir_all(d).map_err(|e| format!("{} を作れません: {e}", d.display()))?;
    }
    let recommended = work_root.join("tmux-recommended.conf");
    let bare = work_root.join("tmux-bare.conf");
    fs::write(&recommended, RECOMMENDED).map_err(|e| e.to_string())?;
    fs::write(&bare, BARE).map_err(|e| e.to_string())?;

    println!("▶ accept-linux (docs/compat/checklist.md の A〜G と L の自動の部分)");
    let mut rows: Vec<(String, String, String, String)> = Vec::new();
    let (l1, ctx) = match l1_install(&archive, home, work, recommended, bare) {
        Ok((note, ctx)) => (Ok(Outcome::Pass(note)), Some(ctx)),
        Err(e) => (Err(e), None),
    };
    record(&mut rows, "L1", "L1 README の Linux の導入手順", l1);
    if let Some(ctx) = &ctx {
        for c in CHECKS {
            if !only.is_empty() && !only.iter().any(|o| o == c.id) {
                continue;
            }
            println!("  {} {}", c.id, c.item);
            let r = (c.run)(ctx);
            record(&mut rows, c.id, c.item, r);
        }
    }

    let report = report(&archive, &rows);
    let out = work_root.join("result.md");
    fs::write(&out, &report).map_err(|e| format!("{} を書けません: {e}", out.display()))?;
    println!("\n{report}\n結果: {}", out.display());
    if !keep {
        let _ = fs::remove_dir_all(work_root.join("work"));
    }
    let failed: Vec<&str> = rows
        .iter()
        .filter(|r| r.2 == "×")
        .map(|r| r.0.as_str())
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "accept-linux: 期待どおりでない項目があります: {}",
            failed.join(", ")
        ))
    }
}

fn find_archive(dir: &Path, triple: &str) -> Result<PathBuf, String> {
    let suffix = format!("-{triple}.tar.gz");
    fs::read_dir(dir)
        .map_err(|e| format!("{} を読めません: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let n = file_name(p);
            n.starts_with("evoeditor-") && n.ends_with(&suffix)
        })
        .ok_or_else(|| format!("{} に *{suffix} がありません", dir.display()))
}

fn record(rows: &mut Vec<(String, String, String, String)>, id: &str, item: &str, r: CheckResult) {
    let (mark, note) = match r {
        Ok(Outcome::Pass(n)) => ("○", n),
        Ok(Outcome::Skip(n)) => ("-", n),
        Err(e) => ("×", e),
    };
    println!("    {mark} {}", note.lines().next().unwrap_or_default());
    if mark == "×" && note.lines().count() > 1 {
        println!("{}", indent(&note));
    }
    rows.push((id.to_owned(), item.to_owned(), mark.to_owned(), note));
}

/// 記録の元 (Markdown)。基準機の固有情報 (ユーザー名を含むパスなど) は書かない。
fn report(archive: &Path, rows: &[(String, String, String, String)]) -> String {
    let sha = sha256_12(archive).unwrap_or_else(|e| e);
    let tmux_v = capture(Command::new("tmux").arg("-V")).unwrap_or_default();
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        })
        .unwrap_or_default();
    let wsl =
        fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"));
    let commit = capture(Command::new("git").args(["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|_| "(git の外)".to_owned());
    let mut s = String::new();
    let _ = writeln!(s, "# accept-linux の結果\n");
    let _ = writeln!(
        s,
        "- コミット / tar.gz の SHA-256 (先頭 12 桁): {} / {sha}",
        commit.trim()
    );
    let _ = writeln!(s, "- アーカイブ: {}", file_name(archive));
    let _ = writeln!(s, "- OS: {os}{}", if wsl { " (WSL)" } else { "" });
    let _ = writeln!(
        s,
        "- 端末: {} (README の推奨設定。L8 は推奨設定なし)",
        tmux_v.trim()
    );
    let _ = writeln!(
        s,
        "\n○ = 期待どおり、× = 期待どおりでない、- = この環境では確かめられない。手で確かめる項目は docs/compat/checklist.md の L を見る。\n"
    );
    let _ = writeln!(s, "| # | 項目 | 結果 | 備考 |\n|---|---|---|---|");
    for (id, item, mark, note) in rows {
        let first = note.lines().next().unwrap_or_default().replace('|', "\\|");
        let _ = writeln!(s, "| {id} | {item} | {mark} | {first} |");
    }
    s
}

fn capture(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn sha256_12(p: &Path) -> Result<String, String> {
    let out = capture(Command::new("sha256sum").arg(p))?;
    Ok(out.chars().take(12).collect())
}

// ---------------------------------------------------------------------------------------------
// L1: README の Linux の導入手順

fn l1_install(
    archive: &Path,
    home: PathBuf,
    work: PathBuf,
    recommended: PathBuf,
    bare: PathBuf,
) -> Result<(String, Ctx), String> {
    let name = file_name(archive);
    let version = name
        .strip_prefix("evoeditor-")
        .and_then(|r| {
            r.split_once("-x86_64-")
                .or_else(|| r.split_once("-aarch64-"))
        })
        .map(|(v, _)| v.to_owned())
        .ok_or_else(|| format!("アーカイブの名前から版が分かりません: {name}"))?;
    let dir = archive.parent().unwrap_or(Path::new("."));
    let mut notes = Vec::new();

    // 2. 検証: 署名 (あれば) とハッシュ
    if dir.join("SHA256SUMS.minisig").exists() {
        let readme =
            fs::read_to_string(workspace_root().join("dist/README.md")).unwrap_or_default();
        let key = readme
            .lines()
            .find_map(|l| l.trim().strip_prefix("minisign -Vm SHA256SUMS -P "))
            .map(str::trim)
            .ok_or("dist/README.md に minisign の公開鍵の行がありません")?;
        match capture(
            Command::new("minisign")
                .args(["-Vm", "SHA256SUMS", "-P", key])
                .current_dir(dir),
        ) {
            Ok(_) => notes.push("署名を検証".to_owned()),
            Err(e) if e.contains("No such file") || e.is_empty() => {
                notes.push("minisign が無いので署名は未検証".to_owned());
            }
            Err(e) => return Err(format!("署名の検証に失敗: {e}")),
        }
    } else {
        notes.push("SHA256SUMS.minisig が無いので署名は未検証 (手元で作った候補)".to_owned());
    }
    let sums = capture(
        Command::new("sha256sum")
            .args(["-c", "SHA256SUMS", "--ignore-missing"])
            .current_dir(dir),
    )
    .map_err(|e| format!("sha256sum -c が失敗: {e}"))?;
    if !sums
        .lines()
        .any(|l| l.starts_with(&name) && l.ends_with(": OK"))
    {
        return Err(format!(
            "SHA256SUMS にアーカイブの OK の行がありません:\n{sums}"
        ));
    }

    // 3〜4. 展開とリンク (README のコマンドのまま、作業用の HOME で)
    let skel = Path::new("/etc/skel/.profile");
    if skel.exists() {
        let _ = fs::copy(skel, home.join(".profile"));
    } else {
        notes.push("/etc/skel/.profile が無い".to_owned());
    }
    let script = format!(
        "set -e\nmkdir -p ~/.local/opt/evoeditor\ntar -xzf '{}' -C ~/.local/opt/evoeditor\nmkdir -p ~/.local/bin\nln -sf ~/.local/opt/evoeditor/evoeditor ~/.local/bin/evoeditor\n",
        archive.display()
    );
    capture(
        Command::new("env")
            .arg(format!("HOME={}", home.display()))
            .args(["sh", "-c", &script]),
    )
    .map_err(|e| format!("展開とリンクに失敗: {e}"))?;

    // 5. ログインし直した端末 (ログインシェル。~/.profile が ~/.local/bin を PATH に入れる) で確認
    let out = capture(
        Command::new("env")
            .args(["-i", &format!("HOME={}", home.display()), "TERM=dumb"])
            .args(["bash", "-lc", "command -v evoeditor && evoeditor --version"]),
    )
    .map_err(|e| format!("ログインし直した端末で evoeditor が見つからないか動きません: {e}"))?;
    let exe = home.join(".local/bin/evoeditor");
    let mut lines = out.lines();
    let found = lines.next().unwrap_or_default();
    if Path::new(found) != exe {
        return Err(format!("command -v evoeditor が {found} を指します"));
    }
    let shown = lines.next().unwrap_or_default();
    if !shown.contains(&version) {
        return Err(format!("--version が版 {version} を含みません: {shown}"));
    }
    notes.insert(
        0,
        format!("ハッシュ OK、ログインシェルの PATH で `{shown}`"),
    );
    Ok((
        notes.join("。"),
        Ctx {
            exe,
            home,
            work,
            recommended,
            bare,
            version,
        },
    ))
}

// ---------------------------------------------------------------------------------------------
// A. 起動と終了

fn a1_start_quit(ctx: &Ctx) -> CheckResult {
    let p = Pane::start(ctx, Profile::Recommended, "a1", (80, 24), &[], &[])?;
    let raw = ctx.fresh("a1-raw.log");
    p.tmux(&["pipe-pane", "-o", &format!("cat >> '{}'", raw.display())])?;
    let s = p.ready()?;
    if !s.contains("無題-1") {
        return Err(format!(
            "ステータスバーに 無題-1 がありません:\n{}",
            indent(&s)
        ));
    }
    p.quit()?;
    if p.format("#{alternate_on}")? != "0" {
        return Err("終了後も代替スクリーンのまま".into());
    }
    pause(200);
    let out = fs::read(&raw).unwrap_or_default();
    let out = String::from_utf8_lossy(&out);
    let tail = out.rfind("\x1b[?1049l").map(|i| &out[i..]);
    match tail {
        None => Err("終了時に代替スクリーンを抜ける ESC[?1049l がありません".into()),
        Some(_) if !out.contains("\x1b[?25h") => {
            Err("終了時にカーソルを出す ESC[?25h がありません".into())
        }
        Some(_) => pass("無題-1 とヒントバー。Ctrl+Q で 0、代替スクリーンを抜けてカーソルを戻す"),
    }
}

fn a3_version_help(ctx: &Ctx) -> CheckResult {
    let v = capture(Command::new(&ctx.exe).arg("--version"))?;
    if !v.contains(&ctx.version) {
        return Err(format!("--version が版 {} を含みません: {v}", ctx.version));
    }
    let h = capture(Command::new(&ctx.exe).arg("--help"))?;
    if !h.contains("--version") {
        return Err(format!("--help に --version がありません:\n{h}"));
    }
    pass(format!("`{}`。--help は終了コード 0", v.trim()))
}

fn a4_new_file(ctx: &Ctx) -> CheckResult {
    let path = ctx.fresh("a4-new.txt");
    let p = Pane::start(
        ctx,
        Profile::Recommended,
        "a4",
        (80, 24),
        &["a4-new.txt"],
        &[],
    )?;
    p.ready()?;
    p.text("new file")?;
    p.wait("入力", 3_000, |s| s.contains("new file"))?;
    p.keys(&["C-s"])?;
    wait_file(&path, 5_000, |b| b.starts_with(b"new file"))?;
    p.quit()?;
    pass("新規として保存できる")
}

fn a5_japanese_name(ctx: &Ctx) -> CheckResult {
    ctx.file("日本語.txt", "日本語の本文です\n".as_bytes())?;
    let p = Pane::start(
        ctx,
        Profile::Recommended,
        "a5",
        (80, 24),
        &["日本語.txt"],
        &[],
    )?;
    p.wait("日本語のファイル名と本文", 5_000, |s| {
        s.contains("日本語の本文です") && s.contains("日本語.txt")
    })?;
    p.quit()?;
    pass("本文とステータスバーの名前が崩れない")
}

// ---------------------------------------------------------------------------------------------
// C. 表示とキー

fn c1_resize(ctx: &Ctx) -> CheckResult {
    let text: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    ctx.file("c1.txt", text.as_bytes())?;
    let p = Pane::start(ctx, Profile::Recommended, "c1", (80, 24), &["c1.txt"], &[])?;
    p.ready()?;
    for (cols, rows) in [(120u16, 40u16), (30, 8), (2, 1), (1, 1), (80, 24)] {
        p.resize(cols, rows)?;
        if cols < 20 {
            pause(500);
            if !p.alive() {
                return Err(format!("{cols}×{rows} で終了しました"));
            }
            continue;
        }
        p.wait(&format!("{cols}×{rows} の描き直し"), 3_000, |s| {
            s.lines().count() == usize::from(rows)
                && status_line(s).is_some_and(|l| width(l) == usize::from(cols))
                && s.contains("line 1")
        })?;
    }
    p.quit()?;
    pass("120×40・30×8・2×1・1×1・80×24 で描き直し、落ちない")
}

fn c2_ctrl_shift(ctx: &Ctx) -> CheckResult {
    ctx.file("c2.txt", b"foo\n")?;
    let p = Pane::start(ctx, Profile::Recommended, "c2", (80, 24), &["c2.txt"], &[])?;
    p.ready()?;
    ctrl_shift_redo(&p)?;
    p.keys(&["C-z"])?;
    p.wait("元に戻す", 2_000, |s| line(s, 0).ends_with("│foo"))?;
    p.quit_discard()?;
    pass("Ctrl+Z で元に戻し、Ctrl+Shift+Z でやり直す")
}

/// `abc` を打ち、Ctrl+Z で消し、Ctrl+Shift+Z で戻す。
fn ctrl_shift_redo(p: &Pane) -> Result<(), String> {
    p.text("abc")?;
    p.wait("入力", 2_000, |s| line(s, 0).ends_with("│abcfoo"))?;
    p.keys(&["C-z"])?;
    p.wait("Ctrl+Z (元に戻す)", 2_000, |s| {
        line(s, 0).ends_with("│foo")
    })?;
    p.keys(&["C-S-z"])?;
    p.wait("Ctrl+Shift+Z (やり直し)", 2_000, |s| line(s, 0).ends_with("│abcfoo"))
        .map_err(|e| {
            format!(
                "Ctrl+Shift+Z がやり直しとして届きません (tmux が Ctrl+Z と区別できる形で送っていない)。{e}"
            )
        })?;
    Ok(())
}

impl Pane {
    /// 未保存の確認が出ても破棄して終わる (確認ダイアログの `D`)。
    fn quit_discard(&self) -> Result<(), String> {
        self.keys(&["C-q"])?;
        pause(300);
        if self.alive() {
            self.keys(&["d"])?;
        }
        self.wait_exit(5_000).map(drop)
    }
}

/// 色と属性の 1 セル分の状態。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
struct Style {
    fg: String,
    bg: String,
    attrs: u16,
}

/// `capture-pane -e` の 1 画面を、行ごとの (文字, 状態) に分ける。SGR 以外のエスケープは読み飛ばす。
fn parse_styled(s: &str) -> (Vec<Vec<(char, Style)>>, Vec<String>) {
    let mut rows = Vec::new();
    let mut sgrs = Vec::new();
    let mut st = Style::default();
    for l in s.lines() {
        let mut row = Vec::new();
        let mut it = l.chars().peekable();
        while let Some(c) = it.next() {
            if c != '\x1b' {
                row.push((c, st.clone()));
                continue;
            }
            if it.peek() != Some(&'[') {
                // OSC など: BEL か ST までを読み飛ばす
                while let Some(d) = it.next() {
                    if d == '\x07' || (d == '\x1b' && it.peek() == Some(&'\\')) {
                        it.next();
                        break;
                    }
                }
                continue;
            }
            it.next();
            let mut params = String::new();
            let mut fin = ' ';
            for d in it.by_ref() {
                if ('\x40'..='\x7e').contains(&d) {
                    fin = d;
                    break;
                }
                params.push(d);
            }
            if fin == 'm' {
                apply_sgr(&mut st, &params);
                sgrs.push(params);
            }
        }
        rows.push(row);
    }
    (rows, sgrs)
}

fn apply_sgr(st: &mut Style, params: &str) {
    let nums: Vec<u16> = params
        .split(';')
        .map(|p| p.split(':').next().unwrap_or("").parse().unwrap_or(0))
        .collect();
    let mut i = 0;
    while i < nums.len() {
        let n = nums[i];
        match n {
            0 => *st = Style::default(),
            1..=9 => st.attrs |= 1 << n,
            21 | 22 => st.attrs &= !(1 << 1 | 1 << 2),
            23..=29 => st.attrs &= !(1 << (n - 20)),
            30..=37 | 90..=97 => st.fg = n.to_string(),
            39 => st.fg.clear(),
            40..=47 | 100..=107 => st.bg = n.to_string(),
            49 => st.bg.clear(),
            38 | 48 | 58 => {
                let take = match nums.get(i + 1) {
                    Some(5) => 2,
                    Some(2) => 4,
                    _ => 0,
                };
                let v: Vec<String> = nums[i + 1..(i + 1 + take).min(nums.len())]
                    .iter()
                    .map(u16::to_string)
                    .collect();
                if n == 38 {
                    st.fg = v.join(";");
                } else if n == 48 {
                    st.bg = v.join(";");
                }
                i += take;
            }
            _ => {}
        }
        i += 1;
    }
}

/// 行 `row` の中の `needle` の各出現の先頭の文字の状態。
fn styles_of(rows: &[Vec<(char, Style)>], needle: &str) -> Vec<Style> {
    let first = needle.chars().next().unwrap_or(' ');
    let n: Vec<char> = needle.chars().collect();
    let mut out = Vec::new();
    for row in rows {
        let chars: Vec<char> = row.iter().map(|c| c.0).collect();
        for i in 0..chars.len() {
            if chars[i] == first && chars[i..].starts_with(&n) {
                out.push(row[i].1.clone());
            }
        }
    }
    out
}

/// 色の段階ごとに、SGR に出てよい色の指定を確かめる。
fn check_color_params(mode: &str, sgrs: &[String]) -> Result<(), String> {
    let mut any_rgb = false;
    for p in sgrs {
        let nums: Vec<u16> = p
            .split(';')
            .map(|x| x.split(':').next().unwrap_or("").parse().unwrap_or(0))
            .collect();
        let mut i = 0;
        while i < nums.len() {
            let n = nums[i];
            let mut idx = 0;
            let (is_color, kind) = match n {
                30..=37 | 40..=47 | 90..=97 | 100..=107 => (true, 16),
                38 | 48 => match nums.get(i + 1) {
                    Some(5) => {
                        idx = nums.get(i + 2).copied().unwrap_or(0);
                        i += 2;
                        (true, 256)
                    }
                    Some(2) => {
                        i += 4;
                        (true, 24)
                    }
                    _ => (true, 0),
                },
                _ => (false, 0),
            };
            if is_color {
                let bad = match mode {
                    "none" => true,
                    "16" => kind == 24 || (kind == 256 && idx >= 16),
                    "256" => kind == 24,
                    _ => false,
                };
                if bad {
                    return Err(format!(
                        "color_mode = {mode} なのに色の指定 ESC[{p}m が出ています"
                    ));
                }
                any_rgb |= kind == 24;
            }
            i += 1;
        }
    }
    if mode == "truecolor" && !any_rgb {
        return Err("color_mode = truecolor なのに 24 ビットの色の指定がありません".into());
    }
    Ok(())
}

fn c5_color_modes(ctx: &Ctx) -> CheckResult {
    ctx.file(
        "c5.txt",
        b"alpha foo beta\nfoo gamma foo\nplain text line\n",
    )?;
    let conf_dir = ctx.home.join(".config/evoeditor");
    fs::create_dir_all(&conf_dir).map_err(|e| e.to_string())?;
    let conf = conf_dir.join("config.toml");
    let modes: [(&str, &str, Envs); 5] = [
        ("truecolor", "truecolor", &[]),
        ("256", "256", &[]),
        ("16", "16", &[]),
        ("none", "none", &[]),
        ("auto", "NO_COLOR", &[("NO_COLOR", "1")]),
    ];
    let result = (|| {
        for (mode, label, envs) in modes {
            fs::write(&conf, format!("[theme]\ncolor_mode = \"{mode}\"\n"))
                .map_err(|e| e.to_string())?;
            let p = Pane::start(ctx, Profile::Recommended, "c5", (80, 16), &["c5.txt"], envs)?;
            p.ready()?;
            let ctx_err = |e: String| format!("{label}: {e}");
            // 選択範囲: 3 行目の先頭の 5 文字
            p.keys(&[
                "Down", "Down", "Home", "S-Right", "S-Right", "S-Right", "S-Right", "S-Right",
            ])?;
            p.wait("選択", 2_000, |s| {
                status_line(s).is_some_and(|l| l.contains("行 3, 桁 6"))
            })
            .map_err(ctx_err)?;
            pause(200);
            let (rows, sgrs1) = parse_styled(&p.screen_styled()?);
            let sel = styles_of(&rows, "plain");
            let normal = styles_of(&rows, "text");
            if sel.first() == normal.first() {
                return Err(ctx_err(format!("選択範囲と本文の見た目が同じ: {sel:?}")));
            }
            // 検索の一致と現在の一致
            p.keys(&["Home", "C-Home", "C-f"])?;
            p.text("foo")?;
            p.wait("検索の件数", 2_000, |s| s.contains("1/3"))
                .map_err(ctx_err)?;
            pause(200);
            let (rows, sgrs2) = parse_styled(&p.screen_styled()?);
            let matches = styles_of(&rows, "foo");
            let body = styles_of(&rows, "alpha");
            // 検索バーの入力欄の "foo" も数えるので、本文の 3 つ (先頭の 3 件) を見る
            let doc: Vec<&Style> = matches.iter().take(3).collect();
            if doc.len() < 3 || doc.iter().any(|m| Some(*m) == body.first()) {
                return Err(ctx_err(format!(
                    "検索の一致と本文の見た目が同じ: {doc:?} / {body:?}"
                )));
            }
            if doc.iter().all(|m| *m == doc[0]) {
                return Err(ctx_err("現在の一致とほかの一致の見た目が同じ".into()));
            }
            let status = rows
                .iter()
                .find(|r| r.iter().map(|c| c.0).collect::<String>().contains(", 桁 "))
                .and_then(|r| r.first().map(|c| c.1.clone()));
            if status.as_ref() == body.first() {
                return Err(ctx_err("ステータスバーと本文の見た目が同じ".into()));
            }
            let mode_key = if label == "NO_COLOR" { "none" } else { mode };
            check_color_params(mode_key, &[sgrs1, sgrs2].concat()).map_err(ctx_err)?;
            p.esc()?;
            p.quit()?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&conf);
    result?;
    pass(
        "5 段階で選択・一致・現在の一致・ステータスバーが本文と区別され、色の指定が段階に収まる (見た目は C4 で目視)",
    )
}

// ---------------------------------------------------------------------------------------------
// D. 貼り付けとクリップボード

fn d1_paste_lines(ctx: &Ctx) -> CheckResult {
    let text: String = (0..100)
        .map(|i| {
            let indent = ["", "    ", "\t", "        "][i % 4];
            format!("{indent}line {i} {{ x }}\n")
        })
        .collect();
    let path = ctx.fresh("d1.txt");
    let p = Pane::start(ctx, Profile::Recommended, "d1", (80, 24), &["d1.txt"], &[])?;
    p.ready()?;
    p.paste(ctx, text.as_bytes())?;
    p.wait("貼り付けの末尾", 5_000, |s| {
        s.contains("line 99 { x }")
    })?;
    p.keys(&["C-s"])?;
    wait_file(&path, 5_000, |b| b == text.as_bytes())?;
    p.keys(&["C-z"])?;
    p.wait("元に戻す", 3_000, |s| !s.contains("line 9"))?;
    p.keys(&["C-s"])?;
    wait_file(&path, 5_000, <[u8]>::is_empty)?;
    p.quit()?;
    pass("インデントのまま 100 行が入り、Ctrl+Z 1 回で消える")
}

fn d3_paste_injection(ctx: &Ctx) -> CheckResult {
    let inj = fs::read(workspace_root().join("docs/compat/paste-injection.txt"))
        .map_err(|e| format!("docs/compat/paste-injection.txt を読めません: {e}"))?;
    let path = ctx.fresh("d3.txt");
    let p = Pane::start(ctx, Profile::Recommended, "d3", (80, 24), &["d3.txt"], &[])?;
    p.ready()?;
    p.paste(ctx, &inj)?;
    p.wait("貼り付けの末尾", 5_000, |s| s.contains("行5: 末尾"))?;
    pause(1_000);
    if path.exists() {
        return Err("貼り付けの中の Ctrl+S で保存された".into());
    }
    if !p.alive() {
        return Err("貼り付けの中の Ctrl+Q で終了した".into());
    }
    p.keys(&["C-s"])?;
    let saved = wait_file(&path, 5_000, |b| !b.is_empty())?;
    let saved = String::from_utf8_lossy(&saved);
    for want in ["行2:", "行3:", "行4:", "行5: 末尾"] {
        if !saved.contains(want) {
            return Err(format!("貼り付けが途中で切れた ({want} がない)"));
        }
    }
    let kept = saved.contains('\x13') && saved.contains("\x1b[201~");
    p.quit()?;
    pass(if kept {
        "保存も終了も起きず、制御文字はそのまま文字として入る"
    } else {
        "保存も終了も起きず、全文が入る (制御文字は置き換えられて入る)"
    })
}

/// Windows の PowerShell を実行する (WSL のとき)。
fn ps(script: &str) -> Result<String, String> {
    capture(Command::new("powershell.exe").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]))
}

fn win_path(p: &Path) -> Result<String, String> {
    Ok(capture(Command::new("wslpath").arg("-w").arg(p))?
        .trim()
        .to_owned())
}

fn d4_clipboard(ctx: &Ctx) -> CheckResult {
    if Command::new("sh")
        .args(["-c", "command -v powershell.exe"])
        .stdout(Stdio::null())
        .status()
        .map_or(true, |s| !s.success())
    {
        return Ok(Outcome::Skip("WSL ではない (powershell.exe が無い)".into()));
    }
    let backup = ctx.work.join("clip-backup.txt");
    let wb = win_path(&backup)?;
    let save_script = format!(
        "$c = Get-Clipboard -Raw; if ($null -eq $c) {{ $c = '' }}; [IO.File]::WriteAllText('{wb}', $c, (New-Object Text.UTF8Encoding $false))"
    );
    ps(&save_script)?;
    let r = d4_round_trip(ctx);
    let _ = ps(&format!(
        "Set-Clipboard -Value ([IO.File]::ReadAllText('{wb}', [Text.Encoding]::UTF8))"
    ));
    r
}

fn d4_round_trip(ctx: &Ctx) -> CheckResult {
    // エディタ → Windows
    let text = "コピー 😀 テスト\n二行目 𠮷\n";
    ctx.file("d4a.txt", text.as_bytes())?;
    let out = ctx.work.join("clip-out.txt");
    let wo = win_path(&out)?;
    let p = Pane::start(ctx, Profile::Recommended, "d4", (80, 24), &["d4a.txt"], &[])?;
    p.ready()?;
    p.keys(&["C-a", "C-c"])?;
    let end = Instant::now() + Duration::from_secs(10);
    let got = loop {
        ps(&format!(
            "[IO.File]::WriteAllText('{wo}', (Get-Clipboard -Raw), (New-Object Text.UTF8Encoding $false))"
        ))?;
        let got = fs::read_to_string(&out)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if got == text || Instant::now() >= end {
            break got;
        }
        pause(300);
    };
    if got != text {
        return Err(format!("Windows のクリップボードの内容が違います: {got:?}"));
    }
    p.quit()?;
    drop(p);

    // Windows → エディタ
    let win_text = "貼り付け 🎉 𠮷\r\n2 行目\r\n";
    let inp = ctx.file("clip-in.txt", win_text.as_bytes())?;
    let wi = win_path(&inp)?;
    ps(&format!(
        "Set-Clipboard -Value ([IO.File]::ReadAllText('{wi}', [Text.Encoding]::UTF8))"
    ))?;
    let path = ctx.fresh("d4b.txt");
    let p = Pane::start(ctx, Profile::Recommended, "d4", (80, 24), &["d4b.txt"], &[])?;
    p.ready()?;
    p.keys(&["C-v"])?;
    p.wait("貼り付け", 10_000, |s| s.contains("2 行目"))?;
    p.keys(&["C-s"])?;
    let saved = wait_file(&path, 5_000, |b| !b.is_empty())?;
    let saved = String::from_utf8_lossy(&saved).replace("\r\n", "\n");
    if saved.trim_end() != win_text.replace("\r\n", "\n").trim_end() {
        return Err(format!("貼り付けた内容が違います: {saved:?}"));
    }
    p.quit()?;
    pass("日本語・絵文字・サロゲートペア・改行が往復で一致 (改行は LF に揃えて比較)")
}

fn d6_big_paste(ctx: &Ctx) -> CheckResult {
    let big: String = (0..65_536)
        .map(|i| format!("paste line {i:06} abcdefghijklmnopqrstuvwxyz0123456789ABCDEF\n"))
        .collect();
    let path = ctx.fresh("d6.txt");
    let p = Pane::start(ctx, Profile::Recommended, "d6", (80, 24), &["d6.txt"], &[])?;
    p.ready()?;
    let t0 = Instant::now();
    p.paste(ctx, big.as_bytes())?;
    p.text("ZMARK")?;
    p.wait("貼り付けの後の入力", 30_000, |s| {
        s.contains("ZMARK")
    })?;
    let secs = t0.elapsed().as_secs_f32();
    p.keys(&["C-s"])?;
    wait_file(&path, 20_000, |b| b.len() == big.len() + 5)?;
    p.quit()?;
    pass(format!(
        "{:.1} MiB を貼り付けて、後に打った文字が {secs:.1} 秒で出る",
        big.len() as f32 / 1_048_576.0
    ))
}

// ---------------------------------------------------------------------------------------------
// E. 検索と置換

fn e1_search(ctx: &Ctx) -> CheckResult {
    ctx.file(
        "e1.txt",
        "foo one\nbar foo\nFoo end\n日本語の行\n".as_bytes(),
    )?;
    let p = Pane::start(ctx, Profile::Recommended, "e1", (80, 24), &["e1.txt"], &[])?;
    p.ready()?;
    p.keys(&["C-f"])?;
    p.text("foo")?;
    // 最初の Enter は、強調だけの現在の一致を確定する (カーソルから探すので同じ一致。03 §12.2、04 §8.4)
    for (key, want) in [
        ("", "1/3"),
        ("Enter", "(3 文字選択)"),
        ("Enter", "2/3"),
        ("Enter", "3/3"),
        ("Enter", "1/3"),
        ("Up", "3/3"),
    ] {
        if !key.is_empty() {
            p.keys(&[key])?;
        }
        p.wait(&format!("件数 {want}"), 2_000, |s| s.contains(want))?;
    }
    p.keys(&["M-c"])?;
    p.wait("大文字小文字を区別して 2 件", 2_000, |s| {
        s.contains("/2")
    })?;
    p.keys(&["BSpace", "BSpace", "BSpace"])?;
    p.text("日本語")?;
    p.wait("日本語の一致", 2_000, |s| s.contains("1/1"))?;
    p.esc()?;
    p.quit()?;
    pass(
        "最初の Enter で現在の一致を確定し、次 (Enter)・前 (↑)・末尾で折り返し・Alt+C・日本語。Shift+Enter は C2 と同じく tmux の拡張キーに依存するので見ない",
    )
}

fn e2_replace_all(ctx: &Ctx) -> CheckResult {
    let orig = b"foo Foo\nfoo\n";
    let path = ctx.file("e2.txt", orig)?;
    let p = Pane::start(ctx, Profile::Recommended, "e2", (80, 24), &["e2.txt"], &[])?;
    p.ready()?;
    p.keys(&["C-r"])?;
    p.text("foo")?;
    p.keys(&["Tab"])?;
    p.text("X")?;
    p.keys(&["M-a"])?;
    p.wait("置換の件数", 3_000, |s| {
        s.contains("3 件を置換しました")
    })?;
    p.esc()?;
    p.keys(&["C-s"])?;
    wait_file(&path, 5_000, |b| b == b"X X\nX\n")?;
    p.keys(&["C-z", "C-s"])?;
    wait_file(&path, 5_000, |b| b == orig)?;
    p.quit()?;
    pass("3 件を置換し、Ctrl+Z 1 回で元に戻る")
}

// ---------------------------------------------------------------------------------------------
// F. ファイルの読み書き

fn f1_crlf(ctx: &Ctx) -> CheckResult {
    let path = ctx.file("f1.txt", "一行目\r\n二行目\r\n".as_bytes())?;
    let p = Pane::start(ctx, Profile::Recommended, "f1", (80, 24), &["f1.txt"], &[])?;
    p.wait("CRLF の表示", 5_000, |s| {
        s.contains(HINT) && s.contains("CRLF")
    })?;
    p.text("X")?;
    p.keys(&["C-s"])?;
    wait_file(&path, 5_000, |b| b == "X一行目\r\n二行目\r\n".as_bytes())?;
    p.quit()?;
    pass("CRLF のまま保存される")
}

fn iconv_sjis(s: &str) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut child = Command::new("iconv")
        .args(["-f", "UTF-8", "-t", "SHIFT_JIS"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("iconv: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("iconv の標準入力")?
        .write_all(s.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(out.stdout)
}

fn f2_shift_jis(ctx: &Ctx) -> CheckResult {
    let path = ctx.file("f2.txt", &iconv_sjis("日本語のテキストです\n")?)?;
    let p = Pane::start(ctx, Profile::Recommended, "f2", (80, 24), &["f2.txt"], &[])?;
    p.wait("Shift_JIS の表示", 5_000, |s| {
        s.contains("日本語のテキストです") && s.contains("Shift_JIS")
    })?;
    p.text("あ")?;
    p.keys(&["C-s"])?;
    let want = iconv_sjis("あ日本語のテキストです\n")?;
    wait_file(&path, 5_000, |b| b == want.as_slice())?;
    p.quit()?;
    pass("Shift_JIS と表示し、Shift_JIS のまま保存する")
}

fn f3_read_only(ctx: &Ctx) -> CheckResult {
    if capture(Command::new("id").arg("-u")).is_ok_and(|u| u.trim() == "0") {
        return Ok(Outcome::Skip(
            "root では読み取り専用の属性が効かない".into(),
        ));
    }
    let path = ctx.file("f3.txt", b"readonly\n")?;
    capture(Command::new("chmod").arg("444").arg(&path))?;
    let p = Pane::start(ctx, Profile::Recommended, "f3", (80, 24), &["f3.txt"], &[])?;
    p.ready()?;
    p.text("Z")?;
    let s = p.wait("編集の拒否", 3_000, |s| {
        status_line(s).is_some_and(|l| l.contains("[RO]")) && s.contains("⚠")
    })?;
    if line(&s, 0).contains("Zreadonly") {
        return Err("読み取り専用なのに編集できた".into());
    }
    p.quit()?;
    pass("編集を理由つきで拒否する")
}

fn f4_external_change(ctx: &Ctx) -> CheckResult {
    let path = ctx.file("f4.txt", b"original\n")?;
    let p = Pane::start(ctx, Profile::Recommended, "f4", (80, 24), &["f4.txt"], &[])?;
    p.ready()?;
    p.text("Y")?;
    pause(1_100);
    fs::write(&path, b"changed outside\n").map_err(|e| e.to_string())?;
    p.keys(&["C-s"])?;
    let s = p.wait("外部の変更の確認", 5_000, |s| {
        s.contains("外部") || s.contains("変更されています")
    })?;
    p.esc()?;
    pause(500);
    if fs::read(&path).map_err(|e| e.to_string())? != b"changed outside\n" {
        return Err(format!(
            "中止したのに書き換えた。確認の画面:\n{}",
            indent(&s)
        ));
    }
    p.quit_discard()?;
    pass("保存の前に確認が出て、Esc で書き換えない")
}

fn f5_quit_cancel(ctx: &Ctx) -> CheckResult {
    ctx.file("f5.txt", b"keep\n")?;
    let p = Pane::start(ctx, Profile::Recommended, "f5", (80, 24), &["f5.txt"], &[])?;
    p.ready()?;
    p.text("Q")?;
    p.keys(&["C-q"])?;
    p.wait("終了の確認", 3_000, |s| {
        s.contains("保存") && s.contains("破棄")
    })?;
    p.esc()?;
    p.wait("キャンセルして編集に戻る", 3_000, |s| {
        line(s, 0).ends_with("│Qkeep") && s.contains(HINT)
    })?;
    if !p.alive() {
        return Err("キャンセルしたのに終了した".into());
    }
    p.quit_discard()?;
    pass("保存 / 破棄 / キャンセルの確認が出て、Esc で何も変わらない")
}

fn f6_overwrite(ctx: &Ctx) -> CheckResult {
    ctx.file("f6a.txt", b"a\n")?;
    let other = ctx.file("f6b.txt", b"b\n")?;
    let p = Pane::start(ctx, Profile::Recommended, "f6", (80, 24), &["f6a.txt"], &[])?;
    p.ready()?;
    p.keys(&["C-k", "s"])?;
    p.wait("名前の入力", 3_000, |s| s.contains("Enter 決定"))?;
    p.keys(&["End"])?;
    for _ in 0..80 {
        p.keys(&["BSpace"])?;
    }
    p.text(&other.display().to_string())?;
    p.keys(&["Enter"])?;
    p.wait("上書きの確認", 3_000, |s| {
        s.contains("既にあります") || s.contains("上書き")
    })?;
    p.esc()?;
    pause(300);
    if fs::read(&other).map_err(|e| e.to_string())? != b"b\n" {
        return Err("取り消したのに上書きした".into());
    }
    p.quit_discard()?;
    pass("既存の名前に上書きの確認が出る")
}

// ---------------------------------------------------------------------------------------------
// G. 巨大ファイル

/// 100 MB の決定的なテキスト (作業ディレクトリに 1 回だけ作る)。
fn big_file(ctx: &Ctx) -> Result<(PathBuf, usize), String> {
    let path = ctx.work.join("g-100mb.txt");
    if let Ok(m) = fs::metadata(&path) {
        return Ok((path, m.len() as usize));
    }
    let mut s = String::with_capacity(100_100_000);
    let mut i = 0u32;
    while s.len() < 100_000_000 {
        let _ = writeln!(
            s,
            "{i:08} The quick brown fox jumps over the lazy dog 0123456789"
        );
        i += 1;
    }
    fs::write(&path, &s).map_err(|e| e.to_string())?;
    Ok((path, s.len()))
}

fn g1_big_cancel(ctx: &Ctx) -> CheckResult {
    big_file(ctx)?;
    let p = Pane::start(
        ctx,
        Profile::Recommended,
        "g1",
        (80, 24),
        &["g-100mb.txt"],
        &[],
    )?;
    let t0 = Instant::now();
    let s = p.wait("進捗か読込の完了", 20_000, |s| {
        s.contains("読込中") || s.contains("00000000 The")
    })?;
    if !s.contains("読込中") {
        let secs = t0.elapsed().as_secs_f32();
        p.quit()?;
        return pass(format!(
            "進捗が出る前に {secs:.1} 秒で読み終わった (取り消しは確かめていない)"
        ));
    }
    let t1 = Instant::now();
    p.keys(&["Escape"])?;
    p.wait("取り消し", 2_000, |s| s.contains("の読込を取り消し"))?;
    let ms = t1.elapsed().as_millis();
    p.quit()?;
    pass(format!("[読込中 n%] が出て、Esc から {ms} ms で取り消す"))
}

fn g2_huge_warning(ctx: &Ctx) -> CheckResult {
    let path = ctx.work.join("g-513mib.txt");
    let f = fs::File::create(&path).map_err(|e| e.to_string())?;
    f.set_len(513 * 1024 * 1024).map_err(|e| e.to_string())?;
    drop(f);
    let p = Pane::start(
        ctx,
        Profile::Recommended,
        "g2",
        (80, 24),
        &["g-513mib.txt"],
        &[],
    )?;
    p.wait("巨大なファイルの警告", 5_000, |s| {
        s.contains("読み取り専用で開く")
    })?;
    p.esc()?;
    pause(500);
    let _ = fs::remove_file(&path);
    if p.alive() {
        p.quit_discard()?;
    }
    pass("開く前に警告が出て、読み取り専用で開くかキャンセルを選べる")
}

fn g3_big_save(ctx: &Ctx) -> CheckResult {
    let (path, len) = big_file(ctx)?;
    let p = Pane::start(
        ctx,
        Profile::Recommended,
        "g3",
        (80, 24),
        &["g-100mb.txt"],
        &[],
    )?;
    p.wait("読込の完了", 60_000, |s| {
        s.contains("00000000 The") && !s.contains("読込中") && s.contains(HINT)
    })?;
    p.text("Q")?;
    p.wait("入力", 3_000, |s| line(s, 0).contains("Q00000000"))?;
    p.keys(&["C-s"])?;
    let t0 = Instant::now();
    p.text("W")?;
    p.wait("保存中の入力", 3_000, |s| {
        line(s, 0).contains("QW00000000")
    })?;
    let echo = t0.elapsed().as_millis();
    let saving = !fs::read(&path)
        .map(|b| b.starts_with(b"Q"))
        .unwrap_or(false);
    wait_file(&path, 60_000, |b| {
        b.len() == len + 1 && b.starts_with(b"Q00000000")
    })?;
    let total = t0.elapsed().as_secs_f32();
    p.quit_discard()?;
    pass(format!(
        "保存 ({total:.1} 秒) の{}に打った文字が {echo} ms で出る",
        if saving { "途中" } else { "直後" }
    ))
}

// ---------------------------------------------------------------------------------------------
// L. Linux

fn l2_sighup(ctx: &Ctx) -> CheckResult {
    let state = ctx.state("l2");
    let _ = fs::remove_dir_all(&state);
    let p = Pane::start(ctx, Profile::Recommended, "l2", (80, 24), &[], &[])?;
    p.ready()?;
    p.text("unsaved L2")?;
    p.wait("入力", 2_000, |s| s.contains("unsaved L2"))?;
    p.tmux(&["kill-session"])?;
    drop(p);
    let recovery = state.join("recovery");
    let end = Instant::now() + Duration::from_secs(10);
    let dir = loop {
        let found = fs::read_dir(&recovery).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).find(|d| {
                fs::read_to_string(d.join("manifest.toml"))
                    .is_ok_and(|m| m.contains("complete = true"))
            })
        });
        if let Some(d) = found {
            break d;
        }
        if Instant::now() >= end {
            return Err(
                "kill-session (SIGHUP) の後 10 秒で complete = true の退避ができません".into(),
            );
        }
        pause(200);
    };
    let body = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| file_name(&e.path()) != "manifest.toml")
        .any(|e| fs::read_to_string(e.path()).is_ok_and(|t| t.contains("unsaved L2")));
    if !body {
        return Err("退避に本文がありません".into());
    }
    let p = Pane::start(ctx, Profile::Recommended, "l2", (80, 24), &[], &[])?;
    p.wait("起動時の退避の案内", 5_000, |s| {
        s.contains("異常終了の時に") || s.contains("そのフォルダを削除してください")
    })?;
    p.quit()?;
    pass(
        "kill-session で recovery/<日時>-<pid>/ に退避し、次の起動で案内する (Windows Terminal のタブを閉じる操作は手で)",
    )
}

fn l3_suspend(ctx: &Ctx) -> CheckResult {
    ctx.file("l3.txt", b"suspend me\n")?;
    let mut argv = env_prefix(ctx, "l3", &[("PS1", "$ ")]);
    argv.extend(["bash", "--norc", "--noprofile", "-i"].map(str::to_owned));
    let p = Pane::start_argv(ctx, Profile::Recommended, (80, 24), &argv)?;
    p.wait("シェル", 3_000, |s| s.contains("$"))?;
    p.text(&format!("{} l3.txt", ctx.exe.display()))?;
    p.keys(&["Enter"])?;
    p.ready()?;
    p.keys(&["C-k", "z"])?;
    p.wait("シェルに戻る", 3_000, |s| {
        s.contains("$") && !s.contains(HINT)
    })?;
    if p.format("#{alternate_on}")? != "0" {
        return Err("サスペンドしても代替スクリーンのまま".into());
    }
    if p.format("#{cursor_flag}")? != "1" {
        return Err("サスペンドしたシェルでカーソルが隠れたまま".into());
    }
    p.resize(100, 30)?;
    pause(300);
    p.text("fg")?;
    p.keys(&["Enter"])?;
    p.wait("今の大きさで描き直し", 3_000, |s| {
        s.lines().count() == 30
            && status_line(s).is_some_and(|l| width(l) == 100)
            && s.contains("suspend me")
    })?;
    if p.format("#{alternate_on}")? != "1" {
        return Err("fg の後に代替スクリーンに戻らない".into());
    }
    p.keys(&["C-q"])?;
    p.wait("シェルに戻る", 3_000, |s| !s.contains(HINT))?;
    pass("端末を戻してシェルへ。fg で代替スクリーンに戻り、100×30 で描き直す")
}

fn l8_bare(ctx: &Ctx) -> CheckResult {
    ctx.file("l8.txt", b"foo\n")?;
    let p = Pane::start(ctx, Profile::Bare, "l8", (80, 24), &["l8.txt"], &[])?;
    p.ready()?;
    p.text("Q")?;
    p.keys(&["C-q"])?;
    p.wait("終了の確認", 3_000, |s| s.contains("破棄"))?;
    let t0 = Instant::now();
    p.keys(&["Escape"])?;
    p.wait("Esc で閉じる", 3_000, |s| s.contains(HINT))?;
    let ms = t0.elapsed().as_millis();
    p.keys(&["C-z"])?;
    p.wait("元に戻す", 2_000, |s| line(s, 0).ends_with("│foo"))?;
    let redo = ctrl_shift_redo(&p).is_ok();
    p.quit_discard()?;
    pass(format!(
        "Esc は {ms} ms で効く (tmux の既定の escape-time)。Ctrl+Shift+Z は{}",
        if redo {
            "やり直しとして届く"
        } else {
            "Ctrl+Z と区別されない (C2 を見る)"
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_の状態を追う() {
        let (rows, sgrs) = parse_styled("a\x1b[1;38;2;1;2;3mb\x1b[22mc\x1b[0md\n\x1b[7me");
        assert_eq!(rows[0][0].1, Style::default());
        assert_eq!(rows[0][1].1.attrs, 1 << 1);
        assert_eq!(rows[0][1].1.fg, "2;1;2;3");
        assert_eq!(rows[0][2].1.attrs, 0);
        assert_eq!(rows[0][2].1.fg, "2;1;2;3");
        assert_eq!(rows[0][3].1, Style::default());
        // 行をまたいで状態が続く
        assert_eq!(rows[1][0].1.attrs, 1 << 7);
        assert_eq!(sgrs.len(), 4);
    }

    #[test]
    fn 色の段階に収まらない指定を見つける() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert!(check_color_params("none", &s(&["1", "7", "0"])).is_ok());
        assert!(check_color_params("none", &s(&["31"])).is_err());
        assert!(check_color_params("16", &s(&["31;44", "1"])).is_ok());
        assert!(check_color_params("16", &s(&["38;5;10"])).is_ok());
        assert!(check_color_params("16", &s(&["38;5;100"])).is_err());
        assert!(check_color_params("256", &s(&["38;5;10"])).is_ok());
        assert!(check_color_params("256", &s(&["48;2;1;2;3"])).is_err());
        assert!(check_color_params("truecolor", &s(&["38;2;1;2;3"])).is_ok());
        assert!(check_color_params("truecolor", &s(&["31"])).is_err());
    }

    #[test]
    fn 幅は全角を_2_と数える() {
        assert_eq!(width("a.txt 行 1, 桁 1"), 16);
        assert_eq!(width("😀a"), 3);
    }

    #[test]
    fn 項目の番号は重ならない() {
        let mut ids: Vec<&str> = CHECKS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CHECKS.len());
    }
}
