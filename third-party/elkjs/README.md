# ELK sources for elkjs 0.9.3

The upstream [0.9.3 release notes](https://github.com/kieler/elkjs/releases/tag/0.9.3)
state **ELK 0.9.1 plus ELK pull request #955**. The source bundle therefore includes:

- elkjs at `a8304cf79fde75bc2ab1a89d28320f53f8637436` (npm 0.9.3 gitHead).
- ELK 0.9.1 at `62d5909f96fad541bc101ad52dabaece6b7eab7e`.
- `elk-955.patch`, the upstream change from
  `fbf2f2d72e5a887e0fb16cdc6eb05730ab0cab51` to
  `cd7d2bb09bda891380b30e15cfd85f6deab678d9`.
  It applies cleanly to the ELK 0.9.1 source.

Extract the two archives as sibling `elkjs/` and `elk/` directories, apply the
included patch inside `elk/`, then follow `elkjs/README.md` and `build.gradle`.
The original Java/JavaScript sources, Gradle wrapper and build instructions are
preserved. These are upstream components; spanseek has not modified them.
The package's EPL-2.0 license and its contributors' notices continue to apply.
