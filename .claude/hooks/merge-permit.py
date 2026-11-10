#!/usr/bin/env python3
"""PR の自己マージの許可フラグを扱う (guard-git.py が読む)。

    python .claude/hooks/merge-permit.py grant --until YYYY-MM-DD [--note TEXT]
    python .claude/hooks/merge-permit.py revoke
    python .claude/hooks/merge-permit.py status

- フラグは `<git の共通ディレクトリ>/eve-allow-pr-merge` (JSON)。commit されず、worktree の間で共有される。
- grant は guard-git.py が必ず確認にする (人間が許可を出す 1 回の確認)。revoke と status は確認しない。
- 許可の中身: 期限 (until の日まで、最長 31 日先)。`develop` 宛ての PR を `gh pr merge --squash` で入れるときだけ効く。
"""
import argparse
import datetime
import json
import os
import subprocess
import sys

FLAG_NAME = "eve-allow-pr-merge"
MAX_DAYS = 31


def flag_path(cwd=None):
    out = subprocess.run(["git", "rev-parse", "--git-common-dir"], cwd=cwd or None,
                         capture_output=True, text=True, timeout=5)
    if out.returncode != 0:
        raise RuntimeError("git リポジトリの中で実行してください")
    common = out.stdout.strip()
    if not os.path.isabs(common):
        common = os.path.join(cwd or os.getcwd(), common)
    return os.path.normpath(os.path.join(common, FLAG_NAME))


def load(cwd=None):
    """有効な許可なら dict、なければ None (期限切れ・壊れたファイルも None)。"""
    try:
        with open(flag_path(cwd), encoding="utf-8") as f:
            data = json.load(f)
        until = datetime.date.fromisoformat(data["until"])
    except Exception:
        return None
    today = datetime.date.today()
    if not (today <= until <= today + datetime.timedelta(days=MAX_DAYS)):
        return None
    return data


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("grant")
    g.add_argument("--until", required=True, help="YYYY-MM-DD (この日まで有効)")
    g.add_argument("--note", default="", help="許可の根拠 (例: v0.3 の M12〜M16)")
    sub.add_parser("revoke")
    sub.add_parser("status")
    a = p.parse_args()

    path = flag_path()
    if a.cmd == "grant":
        until = datetime.date.fromisoformat(a.until)
        today = datetime.date.today()
        if until < today or until > today + datetime.timedelta(days=MAX_DAYS):
            sys.exit(f"--until は今日から {MAX_DAYS} 日以内の日付にしてください")
        data = {"until": until.isoformat(), "granted": today.isoformat(), "note": a.note,
                "base": "develop", "method": "squash"}
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            json.dump(data, f, ensure_ascii=False)
            f.write("\n")
        print(f"許可しました: {until} まで、develop 宛ての PR の squash マージ ({path})")
    elif a.cmd == "revoke":
        if os.path.exists(path):
            os.remove(path)
            print("許可を取り消しました")
        else:
            print("許可は出ていません")
    else:
        data = load()
        print(f"有効: {data['until']} まで ({data.get('note', '')})" if data else "許可は出ていません (または期限切れ)")


if __name__ == "__main__":
    main()
