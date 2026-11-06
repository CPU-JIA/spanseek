# License texts omitted from package archives

`manifest.json` records exact package versions, source URLs and SHA-256 for
license/notice files that upstream package archives omitted. Most come from the
repository commit identified by the published crate's `.cargo_vcs_info.json`.

Platform-specific npm bindings reuse their parent package's license files at the
exact same version. The manifest records this relationship. `isarray` contains
its full license in its original README. `format` retains its original README
copyright/declaration together with the standard MIT terms. Standard CC0/MPL
texts supplement packages whose archives contain only the license declaration;
the exact covered MPL source archives also accompany the release.

`khroma` declares no package.json license field, but includes a full MIT license.
Its resolution is tied to the hash of that actual file.

`scripts/build-license-bundle.py` verifies these files, collects installed
package license/notice text and produces the installer resources and release
attachments. It fails for missing evidence or changed source obligations.
