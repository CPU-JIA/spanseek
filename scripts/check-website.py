"""Check tracked website links, caption timing and real media decoding without a desktop build."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit
import json
import re
import subprocess
import imageio_ffmpeg

ROOT = Path(__file__).resolve().parent.parent


class Page(HTMLParser):
    def __init__(self, text):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if attrs.get("id"):
            self.ids.add(attrs["id"])
        for attribute in ("href", "src", "poster"):
            if attrs.get(attribute):
                self.links.append(attrs[attribute])


def main():
    tracked = set(subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode("utf-8").split("\0"))
    pages = {name: Page((ROOT / name).read_text("utf-8")) for name in tracked
             if name.startswith(("docs/download/", "docs/guide/")) and name.endswith(".html") and not name.endswith("doc-template.html")}
    version = subprocess.check_output(["node", "scripts/check-version.mjs"], cwd=ROOT, text=True).strip()
    generated = {f"docs/download/downloads/spanseek_{version}_x64-setup.exe", "docs/download/downloads/SHA256SUMS.txt"}
    missing = []
    checked = 0
    for name, page in pages.items():
        for link in page.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            checked += 1
            target = ((ROOT / name).parent / unquote(url.path)).resolve() if url.path else ROOT / name
            relative = target.relative_to(ROOT).as_posix() if target.is_relative_to(ROOT) else None
            if relative in generated:
                continue  # Validated against BUILD-INFO + hashes by prepare-download-site.ps1.
            if relative not in tracked or not target.is_file():
                missing.append({"page": name, "link": link})
            elif url.fragment and relative in pages and unquote(url.fragment) not in pages[relative].ids:
                missing.append({"page": name, "link": link, "reason": "missing anchor"})
    if missing:
        raise ValueError(json.dumps(missing, ensure_ascii=True))

    cue_file = ROOT / "docs/download/assets/brand-mg.zh.vtt"
    cues = cue_file.read_text("utf-8-sig").replace("\r\n", "\n")
    if not cues.startswith("WEBVTT\n"):
        raise ValueError("Missing WebVTT header")
    cue_times = re.findall(r"^(\d\d:\d\d:\d\d\.\d{3}) --> (\d\d:\d\d:\d\d\.\d{3})[^\n]*\n([^\n]+)", cues, re.MULTILINE)
    if not cue_times:
        raise ValueError("No captions")
    def seconds(value):
        hours, minutes, rest = value.split(":")
        return int(hours) * 3600 + int(minutes) * 60 + float(rest)
    previous = 0
    for start, end, text in cue_times:
        if not text.strip() or not previous <= seconds(start) < seconds(end) <= 50:
            raise ValueError("Invalid, overlapping, or out-of-film caption timing")
        previous = seconds(end)
    for name in sorted(tracked):
        if name.startswith("docs/download/") and name.endswith(".js"):
            subprocess.run(["node", "--check", name], cwd=ROOT, check=True)
    for extension in ("mp4", "webm"):
        subprocess.run([imageio_ffmpeg.get_ffmpeg_exe(), "-hide_banner", "-v", "error", "-xerror", "-i",
                        f"docs/download/assets/brand-mg.{extension}", "-f", "null", "-"], cwd=ROOT, check=True)
    print(json.dumps({"html_pages": len(pages), "local_links": checked, "captions": len(cue_times), "decoded_media": 2, "missing": 0}))


if __name__ == "__main__":
    main()
