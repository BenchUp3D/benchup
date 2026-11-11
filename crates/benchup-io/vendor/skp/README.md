# SketchUp reader dependencies

These are unmodified published JavaScript files, embedded by `src/interchange.rs`
and used in a private module scope. Browser BenchUp runs them in a dedicated Web
Worker. Desktop BenchUp uses the equivalent compiled C++ reader described below
on the file-operation thread. No npm install, bundler, external service, SketchUp
SDK, or Python/Node installation is needed at runtime.

- OpenSKP **1.3.0**: npm `openskp`, `dist/index.js` → `openskp.cjs`, MIT.
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
is changed.

## Native reader

The native sources, C ABI, and safe Rust ownership boundary live in
[`benchup-skp`](../../../benchup-skp/README.md). Its build script is independent
of application server/update configuration and compiles C/C++ files in parallel.
Browser builds use only the JavaScript files here and bypass native compilation
entirely. No interpreter or JavaScript engine is linked on desktop.
