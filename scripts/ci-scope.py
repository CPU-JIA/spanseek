"""Choose checks from the actual Git diff; unknown inputs always require a full build."""
from pathlib import Path
import argparse
import json
import os
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
# Installer/license inputs deliberately do not appear here.
DOC_FILES = {"README.md", "docs/README.md", "docs/development.md", "docs/releasing.md",
             "docs/quick-start.md", "docs/user-guide.md", "docs/known-issues.md",
             "docs/privacy.md", "docs/format-support-matrix.md", "docs/external-api.md",
             "docs/external-capabilities.md"}
SITE_SUFFIXES = {".html", ".css", ".js", ".svg", ".ico", ".png", ".jpg", ".webm", ".mp4", ".vtt"}
SHA = re.compile(r"[0-9a-f]{40}")


def website_only(paths):
    return bool(paths) and all(
        path in DOC_FILES or (
            path.startswith(("docs/download/", "docs/guide/"))
            and Path(path).suffix in SITE_SUFFIXES
        ) for path in paths
    )


def changed_paths(event_name, event, git):
    if event_name == "pull_request":
        before, after = event["pull_request"]["base"]["sha"], event["pull_request"]["head"]["sha"]
    elif event_name == "push":
        before, after = event["before"], event["after"]
    else:
        raise ValueError("manual or unknown event requires complete validation")
    if not all(isinstance(value, str) and SHA.fullmatch(value) and value != "0" * 40 for value in (before, after)):
        raise ValueError("missing or invalid diff boundary")
    if event_name == "pull_request":
        before = git("merge-base", before, after).decode().strip()
        if not SHA.fullmatch(before):
            raise ValueError("cannot resolve merge base")
    # Renames are deletions + additions: moving a build input into docs must not bypass checks.
    return [name for name in git("diff", "--no-ext-diff", "--no-renames", "--name-only", "-z", before, after, "--").decode("utf-8").split("\0") if name]


def scope(event_name, event, git):
    try:
        paths = changed_paths(event_name, event, git)
        return {"full": not website_only(paths), "paths": paths, "reason": "changed files"}
    except (KeyError, TypeError, ValueError, OSError, subprocess.CalledProcessError) as failure:
        return {"full": True, "paths": [], "reason": str(failure)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch"))
    parser.add_argument("--event-path", default=os.environ.get("GITHUB_EVENT_PATH"))
    args = parser.parse_args()
    try:
        event = json.loads(Path(args.event_path).read_text("utf-8")) if args.event_path else {}
    except (OSError, ValueError):
        event = {}
    result = scope(args.event, event, lambda *args: subprocess.check_output(["git", *args], cwd=ROOT))
    print(json.dumps(result, ensure_ascii=True))
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(f"full={str(result['full']).lower()}\n")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
            output.write("### Check scope\n\n" + ("Complete desktop checks and applicable installer build." if result["full"] else "Website and user-document checks; desktop build inputs unchanged.") + "\n")


if __name__ == "__main__":
    main()
