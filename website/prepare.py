"""Build the public static site from tracked pages and a verified release."""
from html import escape
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit
import argparse
import hashlib
import json
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
RELEASE_BASE = "https://github.com/CPU-JIA/spanseek/releases/download"
PUBLIC_SUFFIXES = {".html", ".css", ".js", ".svg", ".ico", ".png", ".jpg", ".webm", ".mp4", ".vtt"}


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


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
        for key in ("href", "src", "poster", "data-dark-src", "data-light", "data-dark"):
            if attrs.get(key):
                self.links.append(attrs[key])


def prepare(release, output, origin):
    origin_url = urlsplit(origin)
    if origin_url.scheme != "https" or not origin_url.netloc or origin_url.path not in ("", "/") or origin_url.query or origin_url.fragment:
        raise ValueError("Origin must be an HTTPS origin without a path, query or fragment")
    origin = origin.rstrip("/")
    if output.exists():
        raise ValueError("Use a new output directory; existing deployments are never overwritten")
    info = json.loads((release / "BUILD-INFO.json").read_text("utf-8-sig"))
    version = subprocess.check_output(["node", "scripts/check-version.mjs"], cwd=ROOT, text=True).strip()
    if info["version"] != version or info["installer"] != f"spanseek_{version}_x64-setup.exe":
        raise ValueError("Source and installer versions differ")
    manifest = {}
    for line in (release / "SHA256SUMS.txt").read_text("utf-8-sig").splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)", line)
        if not match or match[2] in manifest:
            raise ValueError("Invalid or duplicate release checksum entry")
        expected, name = match.groups()
        if sha256(release / name) != expected.lower():
            raise ValueError(f"Release checksum mismatch: {name}")
        manifest[name] = expected.lower()
    if not {"BUILD-INFO.json", info["installer"]} <= manifest.keys():
        raise ValueError("Checksum list must cover metadata and installer")
    if manifest[info["installer"]] != info["sha256"].lower() or (release / info["installer"]).stat().st_size != info["bytes"]:
        raise ValueError("Installer does not match BUILD-INFO.json")

    tracked = subprocess.check_output(["git", "ls-files", "-z", "--", "docs/download", "docs/guide"], cwd=ROOT).decode("utf-8").split("\0")
    output.mkdir(parents=True)
    templates = output.with_name(output.name + "-templates")
    if templates.exists():
        raise ValueError("Release template output already exists")
    templates.mkdir()
    copied = []
    for name in filter(None, tracked):
        source = ROOT / name
        if source.name == "doc-template.html":
            continue
        if source.suffix not in PUBLIC_SUFFIXES:
            raise ValueError(f"Unrecognized public asset: {name}")
        relative = Path(name).relative_to("docs/download") if name.startswith("docs/download/") else Path("guide") / Path(name).relative_to("docs/guide")
        target = output / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.suffix == ".html":
            html = source.read_text("utf-8")
            html = html.replace("../guide/", "guide/") if name.startswith("docs/download/") else html.replace("../download/", "../")
            if source.name == "install.html":
                replacements = {
                    "__INSTALLER_SIZE__": f"{info['bytes'] / (1024 * 1024):.2f} MiB",
                    "__INSTALLER_DATE__": info["built_at"][:10],
                    "__INSTALLER_SHA256__": info["sha256"].upper(),
                }
                for token, value in replacements.items():
                    if html.count(token) != 1:
                        raise ValueError(f"Expected one installation token: {token}")
                    html = html.replace(token, value)
            if re.search(r"__[A-Z_]+__", html):
                raise ValueError(f"Unfilled public template: {name}")
            # Keep download provenance explicit; do not bundle the 93 MiB installer as a static asset.
            html = html.replace('href="downloads/', f'href="{RELEASE_BASE}/v{version}/')
            canonical = origin + ("/" if relative.as_posix() == "index.html" else "/" + relative.as_posix())
            html = html.replace("</head>", f'    <link rel="canonical" href="{escape(canonical, quote=True)}" />\n  </head>', 1)
            target.write_text(html, encoding="utf-8", newline="\n")
            if relative.as_posix() in ("index.html", "install.html"):
                template = html.replace(version, "__RELEASE_VERSION__")
                template = template.replace("安装包当前未签名", "__RELEASE_SIGNATURE__").replace("安装包未签名", "__RELEASE_SIGNATURE__")
                if relative.as_posix() == "install.html":
                    for value, token in ((replacements["__INSTALLER_SIZE__"], "__RELEASE_SIZE__"),
                                         (replacements["__INSTALLER_DATE__"], "__RELEASE_DATE__"),
                                         (info["sha256"].upper(), "__RELEASE_SHA256__")):
                        if template.count(value) != 1:
                            raise ValueError(f"Expected one release field: {value}")
                        template = template.replace(value, token)
                (templates / relative.name).write_text(template, encoding="utf-8", newline="\n")
        else:
            shutil.copyfile(source, target)
        copied.append(relative.as_posix())

    # Keep release downloads external and preserve historical site URLs.
    download_names = sorted(set(manifest) | {"SHA256SUMS.txt"})
    caddy = """spanseek.com {
    root * /data/spanseek/current
    redir /index.html / permanent
    encode zstd gzip
    header X-Content-Type-Options nosniff
    header Referrer-Policy strict-origin-when-cross-origin
    @release_pages path / /index.html /install.html /release.json /downloads/SHA256SUMS.txt
    header @release_pages Cache-Control no-cache
    handle_path /docs/download/* {
        redir {uri} permanent
    }
    handle_path /docs/guide/* {
        redir /guide{uri} permanent
    }
"""
    caddy += "".join(f"    redir /downloads/{name} {RELEASE_BASE}/v{version}/{name} 302\n" for name in download_names if name != "SHA256SUMS.txt")
    caddy += """    file_server
}

www.spanseek.com {
    redir https://spanseek.com{uri} permanent
}
"""
    output.with_name(output.name + ".Caddyfile").write_text(caddy, encoding="utf-8")
    (output / "robots.txt").write_text(f"User-agent: *\nAllow: /\nSitemap: {origin}/sitemap.xml\n", encoding="utf-8")
    urls = [origin + ("/" if name == "index.html" else "/" + name) for name in sorted(copied) if name.endswith(".html")]
    sitemap = '<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
    sitemap += "".join(f"  <url><loc>{escape(url)}</loc></url>\n" for url in urls) + "</urlset>\n"
    (output / "sitemap.xml").write_text(sitemap, encoding="utf-8")

    pages = {name: Page((output / name).read_text("utf-8")) for name in copied if name.endswith(".html")}
    links = 0
    for name, page in pages.items():
        for link in page.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            links += 1
            if url.path.startswith("/"):
                target = (output / unquote(url.path).lstrip("/")).resolve()
            else:
                target = ((output / name).parent / unquote(url.path)).resolve() if url.path else output / name
            if target == output:
                target = output / "index.html"
            if not target.is_relative_to(output) or not target.is_file():
                raise ValueError(f"Missing or out-of-site link: {name} -> {link}")
            relative = target.relative_to(output).as_posix()
            if url.fragment and relative in pages and unquote(url.fragment) not in pages[relative].ids:
                raise ValueError(f"Missing anchor: {name} -> {link}")
    files = [{"path": p.relative_to(output).as_posix(), "bytes": p.stat().st_size, "sha256": sha256(p)} for p in sorted(output.rglob("*")) if p.is_file()]
    report = {"version": version, "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "origin": origin, "installer_sha256": info["sha256"], "html_pages": len(pages), "local_links": links, "bytes": sum(p["bytes"] for p in files), "files": files}
    output.with_name(output.name + "-manifest.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "files"}, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-directory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--origin", default="https://spanseek.com")
    args = parser.parse_args()
    prepare(args.release_directory.resolve(strict=True), args.output.resolve(), args.origin)
