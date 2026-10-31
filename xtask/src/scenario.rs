//! `xtask scenario run [<部分一致>] [--print-frame]` / `xtask scenario check` (docs/11 §6.3、NFR-707)。
//! ランナーは eve-testkit (`eve_testkit::scenario`) にあり、ここは eve-testkit の試験 `scenarios` を環境変数で絞って呼ぶだけ
//! (xtask を製品の crate に依存させないため)。

use std::process::Command;

use crate::util::run;

pub fn run_cmd(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let sub = it.next().map(String::as_str);
    let mut cmd = Command::new("cargo");
    cmd.args([
        "test",
        "-q",
        "-p",
        "eve-testkit",
        "--test",
        "scenarios",
        "--",
    ]);
    cmd.args(["シナリオはすべて検証を通り_期待どおりに動く", "--nocapture"]);
    match sub {
        Some("check") => {
            cmd.env("EVE_SCENARIO_CHECK", "1");
            run("scenario check", &mut cmd)
        }
        Some("run") => {
            for a in it {
                match a.as_str() {
                    "--print-frame" => {
                        cmd.env("EVE_SCENARIO_PRINT_FRAME", "1");
                    }
                    f if !f.starts_with("--") => {
                        cmd.env("EVE_SCENARIO", f);
                    }
                    other => return Err(format!("scenario run: 知らない引数 `{other}`")),
                }
            }
            run("scenario run", &mut cmd)
        }
        _ => Err(
            "使い方: cargo xtask scenario run [<部分一致>] [--print-frame] | scenario check"
                .to_owned(),
        ),
    }
}
