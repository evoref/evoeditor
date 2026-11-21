//! ワークスペースの木を決定的に作る (docs/09 §4 の「ワークスペースの木」、03 §12.7.8)。`cargo xtask gen-corpus --workspace` と、
//! gitignore の git との差分試験 (`tests/gitignore_git.rs`) が `#[path]` で取り込んで同じ木を作る (標準ライブラリだけを使う)。
//!
//! 木の形は S-26 の木 (`spikes/S-26/REPORT.md` の「木」) を引き継ぐ: 除外される大きな木 (`target/`・`web/node_modules/`・
//! `vendor/`・`.git/info/exclude` で除外する `scratch/`)、1 つのディレクトリに多くのファイル (`fixtures/flat/`)、40 段の
//! 入れ子 (パスが 260 文字を超える)、約 180 バイトの名前、日本語の名前、ルートと入れ子の `.gitignore` の雛形、パターンと
//! 大文字小文字だけが違う名前、入れ子のリポジトリ 1 つ、`.git` がファイルの worktree 1 つ (共通の git ディレクトリの
//! `commondir` を辿る)。横断検索の計測のために、中身の一部を Shift_JIS・EUC-JP・UTF-16 にし、`max_file_size` (既定 2 MiB) を
//! 超えるファイルとバイナリを少し混ぜる。名前の大文字小文字だけが違う 2 つのファイルは作らない (NTFS・APFS で作れない)。
//! 内容はすべて合成したもので、実在の文書は使わない。
//!
//! リポジトリは git で作る (`git init`・`git worktree add`)。git はシステム・利用者の設定を読まない形で呼ぶ
//! (`GIT_CONFIG_NOSYSTEM=1`・`GIT_CONFIG_GLOBAL` に空のファイル)。worktree の本体のリポジトリは木の外 (`<木>-gitdirs/`) に置く。

#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 木の大きさ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeSize {
    /// 約 2,000 ファイル (毎 PR の差分試験)。
    Small,
    /// 約 16 万ファイル (除外の後に残るもの約 10 万。夜間の差分試験と計測)。
    Large,
}

/// 作った木の要約。
#[derive(Debug, Clone)]
pub struct Tree {
    /// 木のルート (ルートのリポジトリの作業ツリー)。
    pub root: PathBuf,
    /// 木の外に置いた git ディレクトリ (worktree の本体) の置き場所。
    pub gitdirs: PathBuf,
    /// リポジトリのルートの、木のルートからの相対パス (`/` 区切り。ルートは空)。
    pub repos: Vec<String>,
    /// 作ったファイルの数 (`.gitignore` を含む。`.git` の中は含まない)。
    pub files: usize,
    /// 作ったディレクトリの数。
    pub dirs: usize,
}

/// 入れ子のリポジトリ (ルートからの相対パス)。
pub const NESTED_REPO: &str = "nested/inner-repo";
/// `.git` がファイルの worktree (ルートからの相対パス)。
pub const LINKED_WORKTREE: &str = "tools/linked-wt";

struct Scale {
    target: usize,
    node_modules: usize,
    vendor: usize,
    scratch: usize,
    flat: usize,
    src_dirs: usize,
    huge_files: usize,
}

const SMALL: Scale = Scale {
    target: 300,
    node_modules: 100,
    vendor: 30,
    scratch: 20,
    flat: 200,
    src_dirs: 90,
    huge_files: 1,
};

const LARGE: Scale = Scale {
    target: 25_000,
    node_modules: 6_000,
    vendor: 500,
    scratch: 200,
    flat: 15_000,
    src_dirs: 10_000,
    huge_files: 3,
};

// ---------- 決定的な乱数 (splitmix64) ----------

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % n as u64).expect("n 未満")
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }

    /// 確率 `num / den` で真。
    fn chance(&mut self, num: usize, den: usize) -> bool {
        self.below(den) < num
    }
}

const WORDS: &[&str] = &[
    "alpha", "bravo", "charlie", "delta", "echo", "editor", "handler", "buffer", "parser",
    "widget", "config", "render", "view", "model", "core", "util", "test", "main", "lib", "index",
    "item", "node", "tree", "path",
];

/// 拡張子 (除外される `log`・`tmp`・`o`・`a`・`min.js` は全体の約 1 割。残るファイルが S-26 の木と同じく約 2/3 になるように)。
const EXTS: &[&str] = &[
    "rs", "rs", "rs", "rs", "rs", "rs", "txt", "txt", "txt", "md", "md", "toml", "toml", "json",
    "json", "c", "h", "py", "cfg", "dat", "bak", "swp", "ts", "go", "java", "html", "css", "yml",
    "sh", "rb", "log", "tmp", "o", "a", "min.js",
];

/// ルートの `.gitignore` (否定・ディレクトリだけ・置き場所への固定・`**/`・`/**`・`a/**/b`・文字の集合・POSIX の文字の集合・
/// `?`・エスケープ・末尾の空白)。
const ROOT_GITIGNORE: &str = "# 合成した規則 (docs/09 §4)\n\
/target/\n\
*.log\n\
!important.log\n\
build/\n\
*.tmp\n\
/vendor/\n\
**/cache/**\n\
docs/**/draft-*.md\n\
[Tt]emp/\n\
*.[oa]\n\
*~\n\
\\#*#\n\
\\!bang.txt\n\
trailing.txt   \n\
keep\\ me\\ \n\
[[:digit:]]*.dat\n\
?.swp\n\
!secret-ok.txt\n\
web/node_modules/\n\
*.min.js\n";

/// ルートのリポジトリの `info/exclude`。
const ROOT_INFO_EXCLUDE: &str = "*.swp\n/scratch/\nsecret*.txt\n";

/// 入れ子の `.gitignore` の雛形 (11 種)。
const TEMPLATES: &[&str] = &[
    // 0: 許可リストの形
    "*\n!*/\n!*.rs\n",
    // 1: 置き場所への固定と名前
    "/local\nlocal-any\n",
    // 2: 除外したディレクトリの中の否定 (効かない)
    "sub/\n!sub/keep.txt\n",
    // 3: 先頭の **/
    "**/gen\n",
    // 4: a/**/b
    "a/**/b.txt\n",
    // 5: 末尾の /** と、ディレクトリの否定
    "out/**\n!out/keep/\n",
    // 6: 文字の集合と POSIX の文字の集合
    "[abc]*.txt\n[!x]?.md\n[[:upper:]]*.cfg\n[[:digit:]][[:alpha:]].txt\n",
    // 7: エスケープ
    "\\#*\n\\!*\nname\\[1\\]\n",
    // 8: BOM と CRLF と末尾の空白
    "\u{feff}*.bak\r\nkeep.bak  \r\n!keep.bak\r\n",
    // 9: POSIX の文字の集合で始まる (globset と git で解釈が違った形、S-26)
    "[[:digit:]]*.txt\n",
    // 10: ? と、途中の * と、否定の否定
    "x?y\n*.gen.*\n!*.gen.keep\n*.gen.keep\n",
];

/// 雛形ごとに置く、照らし方を確かめるファイル (ディレクトリは `/` で区切る)。
const TEMPLATE_PROBES: &[&[&str]] = &[
    &["a.rs", "b.txt", "d/c.rs", "d/e.md"],
    &["local", "local-any", "x/local", "x/local-any", "x/y.txt"],
    &[
        "sub/keep.txt",
        "sub/other.txt",
        "x/sub/keep.txt",
        "keep.txt",
    ],
    &["gen", "x/gen2", "y/gen/z.txt", "y/gen3"],
    &[
        "a/b.txt",
        "a/x/b.txt",
        "a/x/y/b.txt",
        "x/a/b.txt",
        "a/c.txt",
    ],
    &["out/a.txt", "out/keep/b.txt", "out/x/c.txt", "y.txt"],
    &[
        "abc.txt",
        "bcd.txt",
        "xyz.txt",
        "a1.md",
        "x1.md",
        "ab.md",
        "Upper.cfg",
        "lower.cfg",
        "1a.txt",
        "12.txt",
        "A1.txt",
    ],
    &["#hash", "!bang", "name[1]", "name1", "plain.txt"],
    &["a.bak", "keep.bak", "B.BAK", "c.txt"],
    &["1a.txt", "a1.txt", "9.txt", "x.txt"],
    &[
        "xay",
        "xy",
        "x/y",
        "a.gen.c",
        "a.gen.keep",
        "a.gen",
        "b.txt",
    ],
];

/// ignore の規則とパターン、大文字小文字だけが違う名前 (`core.ignorecase` の差を見る)。
const CASE_PROBES: &[&str] = &[
    "case/FOO.LOG",
    "case-l/foo.log",
    "case-b/Build/x.txt",
    "case-c/BUILD/y.txt",
    "case/Readme.TMP",
    "case/X.O",
    "case-l/x.o",
    "case/TEMP/z.txt",
    "case/Temp2/z.txt",
    "case/IMPORTANT.LOG",
    "case/Secret-OK.TXT",
    "case/SECRET.txt",
    "case/Target/t.txt",
    "case/Q.SWP",
    "case/Trailing.TXT",
    "case/Cache/c.txt",
    "case/x/CACHE/c.txt",
    "docs/a/b/draft-2.md",
    "docs/a/b/final.md",
    "docs/Draft-3.md",
];

/// 入れ子のリポジトリの中身 (`.gitignore` と `info/exclude` は内側のもの。外側の `*.log` は効かない)。
const NESTED_GITIGNORE: &str = "*.txt\n!keep.txt\nOUT/\n";
const NESTED_INFO_EXCLUDE: &str = "*.md\n";
const NESTED_FILES: &[&str] = &[
    "a.txt",
    "keep.txt",
    "KEEP.TXT2",
    "inner.log",
    "readme.md",
    "NOTES.MD",
    "out/x.rs",
    "OUT2/z.rs",
    "src/lib.rs",
    "src/a.tmp",
];

/// worktree の中身 (`.gitignore` と、共通の git ディレクトリの `info/exclude`)。
const WORKTREE_GITIGNORE: &str = "*.out\n/bin/\n";
const WORKTREE_INFO_EXCLUDE: &str = "*.cfg\n";
const WORKTREE_FILES: &[&str] = &[
    "a.out",
    "b.rs",
    "c.cfg",
    "D.CFG",
    "bin/x",
    "src/bin/y",
    "notes.log",
    "keep.txt",
];

/// 木を `root` に作る (`root` と `<root>-gitdirs` は、あれば消してから作る)。git が要る。
///
/// # Errors
///
/// ファイルを作れない・git を実行できない・git が失敗したとき。
pub fn generate(root: &Path, size: TreeSize, seed: u64) -> io::Result<Tree> {
    let scale = match size {
        TreeSize::Small => &SMALL,
        TreeSize::Large => &LARGE,
    };
    let gitdirs = gitdirs_of(root);
    remove_if_exists(root)?;
    remove_if_exists(&gitdirs)?;
    fs::create_dir_all(root)?;
    fs::create_dir_all(&gitdirs)?;
    let empty = gitdirs.join("empty-config");
    fs::write(&empty, b"")?;
    let git = Git {
        empty: empty.clone(),
    };

    // リポジトリ (ファイルを置く前に作る。worktree は空のディレクトリにしか作れない)
    git.run(root, &["init", "-q", "."])?;
    let nested = root.join(NESTED_REPO);
    fs::create_dir_all(&nested)?;
    git.run(&nested, &["init", "-q", "."])?;
    let main = gitdirs.join("wt-main");
    fs::create_dir_all(&main)?;
    git.run(&main, &["init", "-q", "."])?;
    git.run(&main, &["commit", "-q", "--allow-empty", "-m", "init"])?;
    let linked = root.join(LINKED_WORKTREE);
    fs::create_dir_all(linked.parent().expect("親"))?;
    let linked_arg = linked.to_string_lossy().into_owned();
    git.run(&main, &["worktree", "add", "-q", "--detach", &linked_arg])?;

    fs::write(root.join(".git/info/exclude"), ROOT_INFO_EXCLUDE)?;
    fs::write(nested.join(".git/info/exclude"), NESTED_INFO_EXCLUDE)?;
    fs::write(main.join(".git/info/exclude"), WORKTREE_INFO_EXCLUDE)?;

    let mut g = Gen {
        root: root.to_path_buf(),
        rng: Rng(seed),
        files: 0,
        dirs: 0,
        made_dirs: HashSet::new(),
    };
    g.file(".gitignore", ROOT_GITIGNORE.as_bytes())?;
    g.bulk("target/debug/deps", "unit", "o", scale.target)?;
    g.bulk("web/node_modules/pkg", "mod", "js", scale.node_modules)?;
    g.bulk("vendor/crate", "src", "rs", scale.vendor)?;
    g.bulk("scratch", "note", "txt", scale.scratch)?;
    g.bulk("fixtures/flat", "case", "txt", scale.flat)?;
    g.fixed()?;
    g.deep()?;
    g.huge(scale.huge_files)?;
    g.random_src(scale.src_dirs)?;
    for f in NESTED_FILES {
        g.file(&format!("{NESTED_REPO}/{f}"), b"inner\n")?;
    }
    g.file(
        &format!("{NESTED_REPO}/.gitignore"),
        NESTED_GITIGNORE.as_bytes(),
    )?;
    for f in WORKTREE_FILES {
        g.file(&format!("{LINKED_WORKTREE}/{f}"), b"wt\n")?;
    }
    g.file(
        &format!("{LINKED_WORKTREE}/.gitignore"),
        WORKTREE_GITIGNORE.as_bytes(),
    )?;
    Ok(Tree {
        root: root.to_path_buf(),
        gitdirs,
        repos: vec![
            String::new(),
            NESTED_REPO.to_owned(),
            LINKED_WORKTREE.to_owned(),
        ],
        files: g.files,
        dirs: g.dirs,
    })
}

/// 木の外に置く git ディレクトリの置き場所 (`<root>-gitdirs`)。
pub fn gitdirs_of(root: &Path) -> PathBuf {
    let mut name = root.file_name().unwrap_or_default().to_os_string();
    name.push("-gitdirs");
    root.with_file_name(name)
}

/// 木と、木の外の git ディレクトリを消す。
///
/// # Errors
///
/// 消せなかったとき。
pub fn remove(root: &Path) -> io::Result<()> {
    remove_if_exists(root)?;
    remove_if_exists(&gitdirs_of(root))
}

fn remove_if_exists(p: &Path) -> io::Result<()> {
    match fs::remove_dir_all(p) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

/// システム・利用者の設定を読まずに git を呼ぶ。
pub struct Git {
    /// 空の設定ファイル (`GIT_CONFIG_GLOBAL` に渡す)。
    pub empty: PathBuf,
}

impl Git {
    /// `cwd` で git を呼ぶ形を作る (環境と、`core.longpaths` などの共通の `-c`)。
    pub fn command(&self, cwd: &Path) -> Command {
        let mut c = Command::new("git");
        c.current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.empty)
            .env("HOME", self.empty.parent().expect("親"))
            .env("XDG_CONFIG_HOME", self.empty.parent().expect("親"));
        for v in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_CEILING_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
        ] {
            c.env_remove(v);
        }
        c.args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.quotePath=false",
            "-c",
            "user.name=gen",
            "-c",
            "user.email=gen@example.invalid",
            "-c",
            "commit.gpgSign=false",
        ]);
        c
    }

    /// git を実行し、失敗なら標準エラーを誤りにして返す。
    ///
    /// # Errors
    ///
    /// 実行できない・終了の状態が失敗のとき。
    pub fn run(&self, cwd: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
        let out = self.command(cwd).args(args).output()?;
        if !out.status.success() {
            return Err(io::Error::other(format!(
                "git {args:?} ({}) が失敗: {}",
                cwd.display(),
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(out.stdout)
    }
}

struct Gen {
    root: PathBuf,
    rng: Rng,
    files: usize,
    dirs: usize,
    made_dirs: HashSet<PathBuf>,
}

impl Gen {
    fn ensure_dir(&mut self, dir: &Path) -> io::Result<()> {
        if self.made_dirs.contains(dir) {
            return Ok(());
        }
        if let Some(parent) = dir.parent()
            && parent != self.root
            && parent.starts_with(&self.root)
        {
            self.ensure_dir(parent)?;
        }
        if !dir.exists() {
            fs::create_dir(dir)?;
            self.dirs += 1;
        }
        self.made_dirs.insert(dir.to_path_buf());
        Ok(())
    }

    fn file(&mut self, rel: &str, body: &[u8]) -> io::Result<()> {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            self.ensure_dir(parent)?;
        }
        fs::write(&path, body)?;
        self.files += 1;
        Ok(())
    }

    fn text(&mut self) -> Vec<u8> {
        let lines = 1 + self.rng.below(3);
        let mut s = String::new();
        for _ in 0..lines {
            let n = 3 + self.rng.below(6);
            let words: Vec<&str> = (0..n).map(|_| self.rng.pick(WORDS)).collect();
            s.push_str(&words.join(" "));
            s.push('\n');
        }
        s.into_bytes()
    }

    /// 中身 (ほとんどは UTF-8。一部を Shift_JIS・EUC-JP・UTF-16・バイナリにする)。
    fn body(&mut self) -> Vec<u8> {
        match self.rng.below(100) {
            0 => b"\x93\xfa\x96\x7b\x8c\xea\x82\xcc\x83\x65\x83\x58\x83\x67 editor\r\n".to_vec(),
            1 => b"\xc6\xfc\xcb\xdc\xb8\xec\xa4\xce\xa5\xc6\xa5\xb9\xa5\xc8 editor\n".to_vec(),
            2 => {
                let mut v = vec![0xFF, 0xFE];
                for u in "日本語のテスト editor\n".encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            }
            3 => b"\x00\x01\x02binary\x00\xff\xfe".to_vec(),
            _ => self.text(),
        }
    }

    fn bulk(&mut self, dir: &str, stem: &str, ext: &str, n: usize) -> io::Result<()> {
        for i in 0..n {
            self.file(&format!("{dir}/{stem}{i:05}.{ext}"), b"x\n")?;
        }
        Ok(())
    }

    /// 雛形・大文字小文字・名前の形の固定の項目。
    fn fixed(&mut self) -> io::Result<()> {
        for (t, (tpl, probes)) in TEMPLATES.iter().zip(TEMPLATE_PROBES).enumerate() {
            let dir = format!("edge/t{t:02}");
            self.file(&format!("{dir}/.gitignore"), tpl.as_bytes())?;
            for p in *probes {
                self.file(&format!("{dir}/{p}"), b"probe\n")?;
            }
        }
        for p in CASE_PROBES {
            self.file(p, b"case\n")?;
        }
        for name in [
            "important.log",
            "other.log",
            "secret.txt",
            "secret-ok.txt",
            "secret2.md",
            "#hash#",
            "!bang.txt",
            "trailing.txt",
            "1.dat",
            "a1.dat",
            "x.swp",
            "notes~",
            "q.min.js",
            ".env",
            ".github/workflows/ci.yml",
            ".eve/settings.toml",
            "local/a.txt",
            "资料/b.txt",
            "資料/仕様書12.txt",
            "資料/メモ.log",
            "テスト/ファイル名.md",
        ] {
            self.file(&format!("misc/{name}"), b"misc\n")?;
        }
        self.file("important.log", b"keep\n")?;
        self.file("trailing.txt", b"t\n")?;
        let long = format!("names/{}.txt", "long-name-".repeat(18));
        self.file(&long, b"long\n")?;
        let long_ja = format!("names/{}.txt", "長い名前".repeat(14));
        self.file(&long_ja, b"long\n")?;
        Ok(())
    }

    /// 40 段の入れ子 (パスは 260 文字を超える)。20 段目に `.gitignore` を置き、その下で効くことを見る。
    fn deep(&mut self) -> io::Result<()> {
        let mut dir = String::from("deep");
        for level in 0..40 {
            dir.push_str(&format!("/level-{level:02}-abcdefgh"));
            if level == 20 {
                self.file(&format!("{dir}/.gitignore"), b"*.tmp2\n/only-here\n")?;
            }
            if level % 5 == 0 {
                self.file(&format!("{dir}/f{level}.txt"), b"deep\n")?;
                self.file(&format!("{dir}/f{level}.tmp2"), b"deep\n")?;
                self.file(&format!("{dir}/only-here"), b"deep\n")?;
            }
        }
        self.file(&format!("{dir}/leaf.txt"), b"leaf\n")?;
        Ok(())
    }

    /// `max_file_size` (既定 2 MiB) を超えるファイル。
    fn huge(&mut self, n: usize) -> io::Result<()> {
        let line = b"editor handler buffer parser widget config render view model core util\n";
        let mut body = Vec::with_capacity(2_621_440 + line.len());
        while body.len() < 2_621_440 {
            body.extend_from_slice(line);
        }
        for i in 0..n {
            self.file(&format!("big/huge{i}.txt"), &body)?;
        }
        Ok(())
    }

    /// ランダムな部分木 (`src/`)。ディレクトリごとに 5〜17 個のファイルを置き、約 2.5% のディレクトリに雛形の `.gitignore` を置く。
    fn random_src(&mut self, n_dirs: usize) -> io::Result<()> {
        const SPECIAL_DIRS: &[&str] = &[
            "build",
            "Build",
            "cache",
            "gen",
            "out",
            "Temp",
            "temp",
            "sub",
            "local",
            "a",
            "node_modules",
            "docs",
            "target",
        ];
        // (相対パス, 深さ, 子の名前 (小文字))
        let mut dirs: Vec<(String, usize, HashSet<String>)> =
            vec![("src".to_owned(), 0, HashSet::new())];
        let mut file_no = 0usize;
        for i in 1..n_dirs {
            // 親は直近のものから選ぶ (深さが偏りすぎないように、深さ 8 まで)
            let lo = dirs.len().saturating_sub(64);
            let mut p = lo + self.rng.below(dirs.len() - lo);
            while dirs[p].1 >= 8 {
                p = self.rng.below(dirs.len());
            }
            let name = if self.rng.chance(1, 40) {
                self.rng.pick(SPECIAL_DIRS).to_owned()
            } else {
                format!("{}{i}", self.rng.pick(WORDS))
            };
            if !dirs[p].2.insert(name.to_ascii_lowercase()) {
                continue;
            }
            let path = format!("{}/{name}", dirs[p].0);
            let depth = dirs[p].1 + 1;
            dirs.push((path, depth, HashSet::new()));
        }
        for (dir, _, names) in &mut dirs {
            if self.rng.chance(1, 40) {
                let t = self.rng.below(TEMPLATES.len());
                self.file(&format!("{dir}/.gitignore"), TEMPLATES[t].as_bytes())?;
                names.insert(".gitignore".to_owned());
            }
            let n = 5 + self.rng.below(13);
            for _ in 0..n {
                file_no += 1;
                let name = self.file_name(file_no);
                if !names.insert(name.to_ascii_lowercase()) {
                    continue;
                }
                let body = self.body();
                self.file(&format!("{dir}/{name}"), &body)?;
            }
        }
        Ok(())
    }

    fn file_name(&mut self, n: usize) -> String {
        let word = self.rng.pick(WORDS);
        let ext = self.rng.pick(EXTS);
        match self.rng.below(40) {
            0 => format!(
                "{}{n}.{}",
                word.to_ascii_uppercase(),
                ext.to_ascii_uppercase()
            ),
            1 => format!("{word}{n}.{ext}~"),
            2 => format!("{}{word}.dat", n % 10),
            3 => format!("#{word}{n}#"),
            4 => format!(
                "{}.swp",
                (b'a' + u8::try_from(n % 26).expect("26 未満")) as char
            ),
            5 => format!("{n}{word}.txt"),
            6 => format!("draft-{n}.md"),
            7 => format!("{word}{n}.gen.c"),
            8 => format!("{word}{n}.gen.keep"),
            9 => format!(
                "x{}y",
                (b'a' + u8::try_from(n % 26).expect("26 未満")) as char
            ),
            10 => format!("Abc{n}.txt"),
            11 => format!("{word}-{n}.Log"),
            _ => format!("{word}{n}.{ext}"),
        }
    }
}
