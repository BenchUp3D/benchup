# SketchUp reader dependencies

These JavaScript files are embedded by `src/interchange.rs`
and used in a private module scope. Browser BenchUp runs them in a dedicated Web
Worker. Desktop BenchUp uses the equivalent compiled C++ reader described below
on the file-operation thread. No npm install, bundler, external service, SketchUp
SDK, or Python/Node installation is needed at runtime.

- OpenSKP **1.5.0 plus upstream fixes**, pinned at `c98561147ea502c6d4d53a257fa4581d0c6cde1d`, MIT.
  Apply `legacy-reader.patch` with `git apply --unidiff-zero`, run the
  TypeScript package's build, and copy `dist/index.js` to `openskp.cjs`.
  The patch ports legacy tag line-style, text, and guide record handling.
  https://github.com/iamahsanmehmood/openskp
- fflate **0.8.3**: npm `fflate`, `lib/browser.cjs` → `fflate.cjs`, MIT.
  https://github.com/101arrowz/fflate
- earcut **3.2.4**: npm `earcut`, `dist/earcut.dev.js` → `earcut.cjs`, ISC.
  https://github.com/mapbox/earcut
- FlatBuffers **25.9.23**: npm `flatbuffers`, `js/flatbuffers.min.js` →
  `flatbuffers.js`, Apache-2.0. https://github.com/google/flatbuffers

The adjacent LICENSE files must accompany redistribution. The adapter is
BenchUp code. The reader supports legacy MFC and modern ZIP/VFF SKP containers.
Textures, scenes, annotations, and linked component definitions are not imported;
component placements become independent editable groups. Texture material base
colors are retained. Malformed or unsupported models fail before the document
is changed. When a modern ZIP directory is damaged, BenchUp can import an intact
CRC-checked geometry record with any recoverable base material metadata.

The `tag-line-styles-2019.skp` and `tag-line-styles-2020.skp` test fixtures come
from OpenSKP's `packages/python/tests/fixtures/sdk_saved_tag_line_styles_*.skp`
at the same commit, under `LICENSE-openskp`.
The `legacy-guide-options.skp` fixture comes from
`packages/rust/tests/fixtures/create_reference/guide_options.skp` at that commit.

## Native reader

The native sources, C ABI, and safe Rust ownership boundary live in
[`benchup-skp`](../../../benchup-skp/README.md). Its build script is independent
of application server/update configuration and compiles C/C++ files in parallel.
Browser builds use only the JavaScript files here and bypass native compilation
entirely. No interpreter or JavaScript engine is linked on desktop.
