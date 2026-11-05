#!/usr/bin/env python3
"""PreToolUse hook: ブランチ運用 (docs/11-dev-workflow.md §9.1) を守らせる。

- main / develop への直接 push・force push・削除を拒否
- main / develop 上での commit / merge / rebase 等を拒否 (作業ブランチ → PR で入れる)
- リリースタグ (v*) の作成・push を拒否 (署名付きタグは人間が付ける)
- git stash を拒否 (作業ツリーを動かす操作)。reset --hard / clean -f / restore / checkout -- は確認を求める
- gh pr merge は確認を求める (merge は人間)。ただし人間が自己マージの許可 (merge-permit.py grant) を
  出していて、develop 宛ての PR を --squash で入れるときだけは確認なしで通す
- 自己マージの許可を出す操作 (merge-permit.py grant、許可のファイルへの直接の書き込み) は必ず確認を求める
- GitHub MCP の merge_pull_request は、許可を見ずに確認を求める (gh pr merge を使う)

標準入力の JSON から tool_name / tool_input を読み、deny / ask / allow のときだけ JSON を出力する。
"""
import importlib.util
import json
import os
import re
import shlex
import subprocess
import sys

sys.dont_write_bytecode = True  # .claude/hooks/__pycache__ を作らない
_spec = importlib.util.spec_from_file_location(
    "merge_permit", os.path.join(os.path.dirname(os.path.abspath(__file__)), "merge-permit.py"))
merge_permit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(merge_permit)

PROTECTED = {"main", "develop"}
TAG_RE = re.compile(r"^(refs/tags/)?v\d")
MUTATING = {"commit", "merge", "rebase", "cherry-pick", "revert", "am", "pull"}
SPLIT_RE = re.compile(r"&&|\|\||[;|\n]")


def decide(kind, reason):
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": kind,
            "permissionDecisionReason": reason,
        }
    }, ensure_ascii=True))
    sys.exit(0)


def current_branch(cwd):
    try:
        out = subprocess.run(
            ["git", "branch", "--show-current"], cwd=cwd or None,
            capture_output=True, text=True, timeout=5)
        return out.stdout.strip()
    except Exception:
        return ""


def resolve(base, path):
    path = path.strip("\"'")
    if re.match(r"^/[a-zA-Z]/", path):  # Git Bash 形式 /c/Users/... -> C:/Users/...
        path = path[1].upper() + ":" + path[2:]
    return os.path.normpath(path if os.path.isabs(path) else os.path.join(base or ".", path))


def git_args(segment):
    """`git [opts] <sub> args...` -> (sub, args, -C で指定されたパス)。git 以外は (None, [], None)。"""
    toks = segment.split()
    for i, t in enumerate(toks):
        if t == "git" or t.endswith("/git") or t.endswith("\\git") or t.lower().endswith("git.exe"):
            rest = toks[i + 1:]
            j = 0
            dash_c = None
            while j < len(rest) and rest[j].startswith("-"):
                if rest[j] == "-C" and j + 1 < len(rest):
                    dash_c = rest[j + 1]
                j += 2 if rest[j] in ("-C", "-c", "--git-dir", "--work-tree") else 1
            if j < len(rest):
                return rest[j], rest[j + 1:], dash_c
            return None, [], dash_c
    return None, [], None


# gh pr merge の、値を取るオプション
GH_MERGE_VALUE_OPTS = {"-R", "--repo", "-t", "--subject", "-b", "--body", "-F", "--body-file",
                       "-A", "--author-email", "--match-head-commit"}
MERGE_ASK = "PR の merge は人間が行います (docs/11 §9.1)。実行してよいか確認してください。"


def check_gh_merge(seg, cwd):
    """自己マージの許可があり、develop 宛ての PR を --squash で入れるときだけ allow。それ以外は ask。"""
    permit = merge_permit.load(cwd)
    if not permit:
        decide("ask", MERGE_ASK)
    try:
        args = shlex.split(seg)[3:]  # `gh pr merge` の後
    except ValueError:
        decide("ask", MERGE_ASK)
    if any(a in ("--admin", "--merge", "-m", "--rebase", "-r", "--disable-auto") for a in args) or \
            not any(a in ("--squash", "-s") for a in args):
        decide("ask", MERGE_ASK + " (自己マージの許可は --squash にだけ効きます)")
    selector, repo, i = [], [], 0
    while i < len(args):
        a = args[i]
        if a in GH_MERGE_VALUE_OPTS:
            if a in ("-R", "--repo") and i + 1 < len(args):
                repo = ["-R", args[i + 1]]
            i += 2
            continue
        if not a.startswith("-"):
            selector.append(a)
        i += 1
    try:
        out = subprocess.run(["gh", "pr", "view", *selector[:1], *repo, "--json", "baseRefName",
                              "-q", ".baseRefName"], cwd=cwd or None,
                             capture_output=True, text=True, timeout=20)
        base = out.stdout.strip() if out.returncode == 0 else ""
    except Exception:
        base = ""
    if base != permit.get("base", "develop"):
        decide("ask", MERGE_ASK + f" (宛て先が {base or '不明'} のため、自己マージの許可は効きません)")
    # allow は、同じコマンドの残りの部分を調べ終えてから main() で出す
    return f"自己マージの許可 ({permit['until']} まで) により、develop 宛ての squash マージを通します。"


def check_permit_command(seg):
    """自己マージの許可を出す操作は必ず確認。revoke / status は通す。"""
    if "merge-permit.py" in seg:
        toks = seg.split()
        if "grant" in toks:
            until = toks[toks.index("--until") + 1] if "--until" in toks[:-1] else "?"
            decide("ask", f"PR の自己マージを許可します ({until} まで、develop 宛ての PR の squash マージ)。"
                          "許可してよいか確認してください。")
        return
    if merge_permit.FLAG_NAME in seg:
        decide("ask", "自己マージの許可のファイルを直接扱う操作です。実行してよいか確認してください。")


def check_push(args, branch):
    flags = [a for a in args if a.startswith("-")]
    pos = [a for a in args if not a.startswith("-")]
    if any(f in ("--all", "--mirror", "--tags") for f in flags):
        decide("deny", "push --all / --mirror / --tags は禁止です (保護ブランチとタグを巻き込むため)。")
    force = any(f in ("-f", "--force", "--delete", "-d") or f.startswith("--force-with-lease") for f in flags)
    refspecs = pos[1:]  # pos[0] は remote
    targets = []
    for r in refspecs:
        if r.startswith("+"):
            force = True
            r = r[1:]
        dst = r.split(":", 1)[1] if ":" in r else r
        if r.startswith(":"):
            force = True
        if dst == "HEAD":
            dst = branch
        targets.append(re.sub(r"^refs/heads/", "", dst))
    if not refspecs:
        targets = [branch]
    for t in targets:
        if TAG_RE.match(t):
            decide("deny", "リリースタグ (v*) の push は人間が行います (署名付きタグ、docs/11 §9.1)。")
    hit = [t for t in targets if t in PROTECTED]
    if hit:
        decide("deny", f"{hit[0]} への直接 push は禁止です。作業ブランチ → PR で入れてください (docs/11 §9.1)。")
    if force:
        decide("deny", "force push と remote ブランチの削除は禁止です (作業ブランチ内の rebase 後の push は人間が行ってください)。")


def main():
    try:
        payload = json.load(sys.stdin)
    except Exception:
        return
    tool = payload.get("tool_name") or ""
    tool_input = payload.get("tool_input") or {}
    if tool.endswith("merge_pull_request"):
        decide("ask", MERGE_ASK + " (MCP の merge は自己マージの許可を見ません)")
    if tool in ("Write", "Edit", "MultiEdit", "NotebookEdit"):
        target = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
        if merge_permit.FLAG_NAME in target:
            decide("ask", "自己マージの許可のファイルを直接書き換えます。許可してよいか確認してください。")
        return
    cmd = tool_input.get("command") or ""
    if merge_permit.FLAG_NAME in cmd or "merge-permit.py" in cmd:
        for seg in SPLIT_RE.split(cmd):
            check_permit_command(seg.strip())
    if not re.search(r"\bgit\b|\bgh\b", cmd):
        return
    cwd = payload.get("cwd")
    allow_reason, other = None, False
    for seg in SPLIT_RE.split(cmd):
        seg = seg.strip()
        if not seg:
            continue
        m = re.match(r"^cd\s+(?:/d\s+)?(\S+)", seg)
        if m:  # `cd <path> && git ...` は、以降の git の対象ディレクトリを変える
            cwd = resolve(cwd, m.group(1))
            continue
        if re.match(r"^gh(\.exe)?\s+pr\s+merge\b", seg):
            allow_reason = check_gh_merge(seg, cwd)
            continue
        other = True
        sub, args, dash_c = git_args(seg)
        if sub is None:
            continue
        # `git -C <path>` や `cd <path> &&` で別のリポジトリを対象にしているときは、そのブランチで判定する
        branch = current_branch(resolve(cwd, dash_c) if dash_c else cwd)
        if sub == "push":
            check_push(args, branch)
        elif sub == "tag":
            names = [a for a in args if not a.startswith("-")]
            if any(TAG_RE.match(n) for n in names) and not any(a in ("-l", "--list") for a in args):
                decide("deny", "リリースタグ (v*) は人間が署名して付けます (docs/11 §9.1)。")
        elif sub == "branch":
            if any(a in ("-d", "-D", "--delete", "-m", "-M", "--move") for a in args) and \
                    any(a in PROTECTED for a in args if not a.startswith("-")):
                decide("deny", "main / develop の削除・改名は禁止です。")
        elif sub == "stash" and not (args and args[0] in ("list", "show")):
            decide("deny", "git stash は使いません (作業ツリーを動かし、改行コードや未追跡の変更を壊しやすい)。"
                           "退避が要るときは、作業ブランチにコミットするか、人間に頼んでください。")
        elif sub == "reset" and "--hard" in args:
            if branch in PROTECTED:
                decide("deny", f"{branch} 上の reset --hard は禁止です。")
            decide("ask", "reset --hard は作業ツリーの変更を捨てます。実行してよいか確認してください。")
        elif sub == "clean" and any(a.startswith("-") and "f" in a for a in args):
            decide("ask", "git clean -f は未追跡のファイルを削除します。実行してよいか確認してください。")
        elif sub == "restore" and "--staged" not in args:
            decide("ask", "git restore は作業ツリーの変更を捨てます。実行してよいか確認してください。")
        elif sub == "checkout" and "--" in args:
            decide("ask", "git checkout -- <path> は作業ツリーの変更を捨てます。実行してよいか確認してください。")
        elif sub in MUTATING:
            if branch == "main":
                decide("deny", "main へ直接コミット・マージはしません。main はリリース専用です (develop → main の PR)。")
            if branch == "develop":
                decide("deny", "develop へ直接コミット・マージはしません。作業ブランチを切って PR (squash) で入れてください (docs/11 §9.1)。")
    if allow_reason:
        # allow はコマンド全体の確認を飛ばすので、gh pr merge (と cd) だけのコマンドに限る
        if other or re.search(r"\$\(|`|[<>]", cmd):
            decide("ask", MERGE_ASK + " (ほかのコマンドと一緒のときは、自己マージの許可は効きません)")
        decide("allow", allow_reason)


if __name__ == "__main__":
    main()
