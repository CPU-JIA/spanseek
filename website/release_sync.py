"""Keep the website download pages in sync with GitHub's latest full release."""
from datetime import datetime, timezone
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

REPOSITORY = "CPU-JIA/spanseek"
API = f"https://api.github.com/repos/{REPOSITORY}/releases/latest"
DOWNLOADS = f"https://github.com/{REPOSITORY}/releases/download"


class MetadataFetchError(ValueError):
    def __init__(self, message, transient=False):
        super().__init__(message)
        self.transient = transient


def request_bytes(url):
    headers = {"User-Agent": "spanseek-website-release-sync", "Accept": "application/vnd.github+json" if url == API else "application/octet-stream"}
    for attempt in range(2):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=20) as response:
                data = response.read(2 * 1024 * 1024 + 1)
            break
        except urllib.error.HTTPError as error:
            if attempt == 0 and error.code in (500, 502, 503, 504):
                time.sleep(2)
                continue
            raise MetadataFetchError(f"GitHub metadata request failed: {urllib.parse.urlsplit(url).path} (HTTP {error.code})", error.code in (500, 502, 503, 504)) from error
        except (urllib.error.URLError, TimeoutError) as error:
            if attempt == 0:
                time.sleep(2)
                continue
            raise MetadataFetchError(f"GitHub metadata request failed: {urllib.parse.urlsplit(url).path} (network error)", True) from error
    if len(data) > 2 * 1024 * 1024:
        raise ValueError("Release metadata exceeds the size limit")
    return data


def select_assets(release):
    tag = release.get("tag_name", "")
    if release.get("draft") is not False or release.get("prerelease") is not False or not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise ValueError("Only a published full semantic-version release is eligible")
    version = tag[1:]
    names = {"BUILD-INFO.json", "SHA256SUMS.txt", f"spanseek_{version}_x64-setup.exe"}
    assets = {}
    for asset in release.get("assets", []):
        name = asset.get("name")
        if name not in names:
            continue
        if name in assets or asset.get("state") != "uploaded":
            raise ValueError("Duplicate or unfinished release asset")
        if asset.get("browser_download_url") != f"{DOWNLOADS}/{tag}/{name}":
            raise ValueError("Release download must belong to the expected repository and tag")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", asset.get("digest") or ""):
            raise ValueError("GitHub did not provide an asset SHA-256 digest")
        if type(asset.get("size")) is not int or asset["size"] <= 0:
            raise ValueError("Invalid release asset size")
        if type(asset.get("id")) is not int or asset["id"] <= 0:
            raise ValueError("Invalid GitHub asset identity")
        assets[name] = asset
    if set(assets) != names:
        raise ValueError("Release is missing the installer or verification metadata")
    fingerprint = hashlib.sha256(json.dumps({"tag": tag, "assets": [{key: assets[name].get(key) for key in ("name", "id", "size", "digest")} for name in sorted(names)]}, sort_keys=True).encode()).hexdigest()
    return version, assets, fingerprint


def validate_release(release, fetch=request_bytes):
    version, assets, fingerprint = select_assets(release)

    def verified(name):
        asset = assets[name]
        try:
            body = fetch(f"https://api.github.com/repos/{REPOSITORY}/releases/assets/{asset['id']}")
        except MetadataFetchError as error:
            if not error.transient:
                raise
            # Both are official GitHub asset URLs; the same size and digest are
            # mandatory regardless of which endpoint delivered the bytes.
            body = fetch(asset["browser_download_url"])
        if len(body) != asset["size"] or "sha256:" + hashlib.sha256(body).hexdigest() != asset["digest"]:
            raise ValueError(f"Downloaded release metadata failed verification: {name}")
        return body

    info_body = verified("BUILD-INFO.json")
    info = json.loads(info_body.decode("utf-8-sig"))
    checksums = {}
    checksums_text = verified("SHA256SUMS.txt").decode("utf-8-sig")
    for line in checksums_text.splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)", line)
        if not match or match[2] in checksums:
            raise ValueError("Invalid checksum manifest")
        checksums[match[2]] = match[1].lower()
    filename = f"spanseek_{version}_x64-setup.exe"
    installer = assets[filename]
    sha = info.get("sha256", "").lower()
    if (info.get("version") != version or info.get("installer") != filename
            or info.get("platform") != "windows-x64" or info.get("bytes") != installer["size"]
            or checksums.get(filename) != sha or installer["digest"] != "sha256:" + sha
            or checksums.get("BUILD-INFO.json") != hashlib.sha256(info_body).hexdigest()):
        raise ValueError("Installer metadata, GitHub digest and checksum manifest disagree")
    if info.get("signature") not in ("NotSigned", "Valid"):
        raise ValueError("Release signature status is not eligible for automatic publication")
    built_at = datetime.fromisoformat(info["built_at"].replace("Z", "+00:00"))
    published_at = datetime.fromisoformat(release["published_at"].replace("Z", "+00:00"))
    if built_at.tzinfo is None or published_at.tzinfo is None:
        raise ValueError("Release dates must include a timezone")
    return {"schema_version": 1, "version": version, "release_id": release["id"],
            "release_url": f"https://github.com/{REPOSITORY}/releases/tag/v{version}",
            "installer_url": installer["browser_download_url"], "checksums_url": "https://spanseek.com/downloads/SHA256SUMS.txt",
            "github_checksums_url": assets["SHA256SUMS.txt"]["browser_download_url"], "_checksums_text": checksums_text,
            "bytes": installer["size"], "sha256": sha.upper(), "signature": info["signature"],
            "built_date": built_at.astimezone(timezone.utc).date().isoformat(),
            "published_at": published_at.astimezone(timezone.utc).isoformat(), "fingerprint": fingerprint}


def render(template, metadata):
    values = {"__RELEASE_VERSION__": metadata["version"],
              "__RELEASE_SIZE__": f"{metadata['bytes'] / (1024 * 1024):.2f} MiB",
              "__RELEASE_DATE__": metadata["built_date"], "__RELEASE_SHA256__": metadata["sha256"],
              "__RELEASE_SIGNATURE__": "安装包未签名" if metadata["signature"] == "NotSigned" else "安装包已进行代码签名"}
    if "__RELEASE_VERSION__" not in template:
        raise ValueError("Missing release-version template marker")
    for key, value in values.items():
        template = template.replace(key, value)
    if re.search(r"__[A-Z_]+__", template):
        raise ValueError("Unresolved release template field")
    return template


def synchronize(root, base, fetch=request_bytes, check=False):
    root, base = root.resolve(strict=True), base.resolve(strict=True)
    if not base.is_relative_to(root / "releases"):
        raise ValueError("Website base must belong to the deployment release directory")
    templates = {name: (base / "templates" / name).read_text("utf-8") for name in ("index.html", "install.html")}
    manifest_bytes = (base / "public-manifest.json").read_bytes()
    template_id = hashlib.sha256(json.dumps(templates, sort_keys=True).encode() + manifest_bytes).hexdigest()
    release = json.loads(fetch(API))
    version, assets, fingerprint = select_assets(release)
    current = root / "current"
    if current.exists() and not current.is_symlink():
        raise ValueError("Refusing to replace a non-symlink website root")
    if current.is_symlink() and not current.resolve(strict=True).is_relative_to(root):
        raise ValueError("Current website escapes its deployment directory")
    previous_path = current / "release.json"
    previous = json.loads(previous_path.read_text("utf-8")) if previous_path.is_file() else {}
    if not check and previous.get("fingerprint") == fingerprint and previous.get("template_id") == template_id:
        return {"status": "unchanged", "version": version}
    metadata = validate_release(release, fetch)
    rendered = {name: render(template, metadata) for name, template in templates.items()}
    if check:
        return {"status": "verified", "version": version, "sha256": metadata["sha256"]}

    manifest = json.loads(manifest_bytes)
    generation = root / "generated" / (version + "-" + uuid.uuid4().hex)
    generation.mkdir(parents=True, mode=0o755)
    for item in manifest["files"]:
        source_path = base / "public" / item["path"]
        source = source_path.resolve(strict=True)
        if not source.is_relative_to(base / "public") or source_path.is_symlink():
            raise ValueError("Public asset escapes its website base")
        if source.stat().st_size != item["bytes"] or hashlib.sha256(source.read_bytes()).hexdigest() != item["sha256"]:
            raise ValueError(f"Website base changed: {item['path']}")
        destination = generation / item["path"]
        if not destination.resolve().is_relative_to(generation):
            raise ValueError("Invalid public asset destination")
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    for name, html in rendered.items():
        (generation / name).write_text(html, encoding="utf-8")
    (generation / "downloads").mkdir(exist_ok=True)
    (generation / "downloads" / "SHA256SUMS.txt").write_text(metadata.pop("_checksums_text"), encoding="utf-8")
    metadata["template_id"] = template_id
    metadata["synchronized_at"] = datetime.now(timezone.utc).isoformat()
    (generation / "release.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temp_link = root / (".current-" + uuid.uuid4().hex)
    temp_link.symlink_to(generation.relative_to(root), target_is_directory=True)
    os.replace(temp_link, current)
    return {"status": "updated", "version": version, "generation": str(generation), "sha256": metadata["sha256"]}


def main():
    import fcntl  # The scheduled updater runs on the Linux web server.
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    with (args.root / ".release-sync.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print(json.dumps({"status": "already_running"}))
            return
        print(json.dumps(synchronize(args.root, args.base, check=args.check), ensure_ascii=False))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Keep the current symlink untouched and make failures visible to systemd.
        print(json.dumps({"status": "failed", "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        sys.exit(1)
