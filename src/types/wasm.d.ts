/**
 * Ambient declaration for wasm files imported with `with { type: "file" }`.
 * Bun resolves these to a path string (embedded by `bun build --compile`).
 */
declare module "*.wasm" {
  const path: string;
  export default path;
}
