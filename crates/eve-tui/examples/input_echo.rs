//! 実コンソールの入力スレッドの診断 (開発用)。`InputEvent` を 1 行ずつ画面とログに出す。F12 で終了。
//!
//!   cargo run -p eve-tui --example input_echo -- --log events.log

#![allow(clippy::print_stdout)]

#[cfg(windows)]
fn main() -> std::io::Result<()> {
    use std::io::Write as _;
    use std::sync::mpsc;

    use eve_tui::win::{ConsoleInput, InputOut, spawn_input_thread};
    use eve_view::{InputEvent, KeyCode};

    struct Tx(mpsc::Sender<Option<InputEvent>>);
    impl InputOut for Tx {
        fn event(&self, ev: InputEvent) -> bool {
            self.0.send(Some(ev)).is_ok()
        }
        fn died(&self) {
            let _ = self.0.send(None);
        }
    }

    let args: Vec<String> = std::env::args().collect();
    let log_path = args
        .iter()
        .position(|a| a == "--log")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let mut log = log_path.map(std::fs::File::create).transpose()?;

    let mut console = ConsoleInput::open()?;
    let (tx, rx) = mpsc::channel();
    let _thread = spawn_input_thread(&console, Tx(tx))?;
    println!("input_echo: キーを押してください (F12 で終了)");
    if let Ok(size) = console.window_size() {
        println!("window size = {}x{}", size.cols, size.rows);
    }
    while let Ok(Some(ev)) = rx.recv() {
        let line = match &ev {
            // 長い本文は先頭だけ
            InputEvent::Paste(s) => format!(
                "Paste(len={}, head={:?})",
                s.len(),
                s.chars().take(24).collect::<String>()
            ),
            InputEvent::Text(s) if s.len() > 64 => format!(
                "Text(len={}, head={:?})",
                s.len(),
                s.chars().take(24).collect::<String>()
            ),
            other => format!("{other:?}"),
        };
        println!("{line}");
        if let Some(f) = log.as_mut() {
            writeln!(f, "{line}")?;
        }
        if let InputEvent::Key(k) = &ev
            && k.code == KeyCode::F(12)
        {
            break;
        }
    }
    console.restore();
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("Windows 専用です");
}
