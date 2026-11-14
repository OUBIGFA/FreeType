"""Capture reviewed working sources in an isolated Git index, never the user's index."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


BUILD_INPUTS = ("Cargo.toml", "Cargo.lock", ".cargo", "core", "windows", "tools", "tests", "third_party")


def git(root, *arguments, env=None):
    result = subprocess.run(
        ["git", "-C", str(root), *arguments], env=env, capture_output=True,
    )
    if result.returncode:
        message = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(message or f"git {arguments[0]} 检查失败（退出码 {result.returncode}）")
    return result.stdout.decode("utf-8").strip()


def capture(index):
    root = Path(git(Path.cwd(), "rev-parse", "--show-toplevel")).resolve()
    index = index.resolve()
    if not index.is_relative_to(root / "_temp") or index.exists():
        raise ValueError("临时索引必须是仓库 _temp 内尚不存在的文件")
    git(root, "check-ignore", "--quiet", str(index))
    untracked = git(root, "ls-files", "--others", "--exclude-standard", "-z", "--", *BUILD_INPUTS)
    if untracked:
        raise ValueError("构建输入尚未纳入索引，请先审阅并暂存：" + ", ".join(untracked.rstrip("\0").split("\0")))
    base = git(root, "rev-parse", "HEAD")
    original = Path(git(root, "rev-parse", "--git-path", "index"))
    if not original.is_absolute():
        original = root / original
    index.parent.mkdir(parents=True, exist_ok=True)
    # 保留已暂存的新文件；只刷新这些已跟踪路径，不把无关未跟踪内容带入构建记录。
    with original.open("rb") as source, index.open("xb") as target:
        shutil.copyfileobj(source, target)
    environment = {**os.environ, "GIT_INDEX_FILE": str(index)}
    git(root, "-c", "core.safecrlf=false", "add", "--update", "--", ".", env=environment)
    return {"baseCommit": base, "tree": git(root, "write-tree", env=environment)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--index", type=Path, required=True)
    parser.add_argument("--expect-tree")
    parser.add_argument("--expect-commit")
    arguments = parser.parse_args()
    try:
        snapshot = capture(arguments.index)
        if ((arguments.expect_tree and snapshot["tree"] != arguments.expect_tree)
                or (arguments.expect_commit and snapshot["baseCommit"] != arguments.expect_commit)):
            raise ValueError("构建期间源码发生变化，不能记录为同一源码快照；请重新构建")
    except (OSError, RuntimeError, ValueError) as error:
        print(f"无法固定构建源码：{error}", file=sys.stderr)
        return 1
    print(json.dumps(snapshot))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
