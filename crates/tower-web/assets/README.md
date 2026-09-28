# Vendored assets

`topcoat-runtime-0.9.0.js` is `browser/dist/index.js` from the
[`topcoat-runtime`](https://crates.io/crates/topcoat-runtime) 0.9.0 crate
(MIT, © Julien Scholz and the Topcoat contributors), unmodified. tower
serves it from memory so the binary needs no asset directory (D§12.3).

When bumping `topcoat`, copy the new crate's `browser/dist/index.js` here
under the new version's name and update `SCRIPT_FILE` in `src/assets.rs`;
the `vendored_runtime_matches_the_pinned_crate` test fails until you do.
